pub async fn get_costs(
    State(state): State<AppState>,
    Query(pagination): Query<super::PaginationQuery>,
) -> impl IntoResponse {
    let (limit, offset) = pagination.resolve();
    let conn = state.db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, model, provider, tokens_in, tokens_out, cost, tier, cached, created_at \
             FROM inference_costs ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| internal_err(&e))?;

    let rows = stmt
        .query_map(rusqlite::params![limit, offset], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "model": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "tokens_in": row.get::<_, i64>(3)?,
                "tokens_out": row.get::<_, i64>(4)?,
                "cost": row.get::<_, f64>(5)?,
                "tier": row.get::<_, Option<String>>(6)?,
                "cached": row.get::<_, i32>(7)? != 0,
                "created_at": row.get::<_, String>(8)?,
            }))
        })
        .map_err(|e| internal_err(&e))?;

    let costs: Vec<Value> = rows
        .filter_map(|r| {
            r.inspect_err(|e| tracing::warn!(error = %e, "skipping corrupted cost row"))
                .ok()
        })
        .collect();
    Ok::<_, JsonError>(axum::Json(json!({ "costs": costs })))
}

#[derive(Deserialize)]
pub struct TimeSeriesQuery {
    pub hours: Option<i64>,
}

pub async fn get_overview_timeseries(
    State(state): State<AppState>,
    Query(params): Query<TimeSeriesQuery>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(24).clamp(1, 168) as usize;
    let conn = state.db.conn();
    let now = chrono::Utc::now().naive_utc();
    let mut labels = Vec::with_capacity(hours);
    let mut cost_per_hour = vec![0.0f64; hours];
    let mut tokens_per_hour = vec![0.0f64; hours];
    let mut sessions_per_hour = vec![0i64; hours];
    let mut latency_samples: Vec<Vec<i64>> = (0..hours).map(|_| Vec::new()).collect();
    let mut cron_success = vec![0.0f64; hours];
    let mut cron_total = vec![0i64; hours];
    let mut cron_ok = vec![0i64; hours];

    for i in 0..hours {
        let hr = (now - chrono::Duration::hours((hours - 1 - i) as i64))
            .format("%H:00")
            .to_string();
        labels.push(hr);
    }

    let parse_ts = |s: &str| -> Option<chrono::NaiveDateTime> {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
    };
    let bucket_for = |ts: chrono::NaiveDateTime| -> Option<usize> {
        let age = now - ts;
        let mins = age.num_minutes();
        if mins < 0 {
            return None;
        }
        let idx_from_end = (mins / 60) as usize;
        if idx_from_end >= hours {
            None
        } else {
            Some(hours - 1 - idx_from_end)
        }
    };

    if let Ok(mut stmt) = conn.prepare(
        "SELECT cost, tokens_in, tokens_out, created_at FROM inference_costs
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        }) {
            for (cost, tin, tout, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    cost_per_hour[idx] += cost;
                    tokens_per_hour[idx] += (tin + tout) as f64;
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT created_at FROM sessions
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| row.get::<_, String>(0)) {
            for created_at in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    sessions_per_hour[idx] += 1;
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT duration_ms, created_at FROM tool_calls
         WHERE created_at >= datetime('now', ?1) AND duration_ms IS NOT NULL",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) {
            for (dur, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    latency_samples[idx].push(dur);
                }
            }
        }
    }

    let mut latency_p50 = vec![0.0f64; hours];
    for i in 0..hours {
        if latency_samples[i].is_empty() {
            continue;
        }
        latency_samples[i].sort_unstable();
        let n = latency_samples[i].len();
        latency_p50[i] = if n % 2 == 1 {
            latency_samples[i][n / 2] as f64
        } else {
            (latency_samples[i][n / 2 - 1] as f64 + latency_samples[i][n / 2] as f64) / 2.0
        };
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT status, created_at FROM cron_runs
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for (status, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    cron_total[idx] += 1;
                    if status == "success" {
                        cron_ok[idx] += 1;
                    }
                }
            }
        }
    }
    for i in 0..hours {
        cron_success[i] = if cron_total[i] > 0 {
            cron_ok[i] as f64 / cron_total[i] as f64
        } else {
            1.0
        };
    }

    Ok::<_, JsonError>(axum::Json(json!({
        "hours": hours,
        "labels": labels,
        "series": {
            "cost_per_hour": cost_per_hour,
            "tokens_per_hour": tokens_per_hour,
            "sessions_per_hour": sessions_per_hour,
            "latency_p50_ms": latency_p50,
            "cron_success_rate": cron_success
        }
    })))
}

pub async fn get_transactions(
    State(state): State<AppState>,
    Query(params): Query<TransactionsQuery>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(24);
    match ironclad_db::metrics::query_transactions(&state.db, hours) {
        Ok(txs) => {
            let items: Vec<Value> = txs
                .into_iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "tx_type": t.tx_type,
                        "amount": t.amount,
                        "currency": t.currency,
                        "counterparty": t.counterparty,
                        "tx_hash": t.tx_hash,
                        "metadata_json": t.metadata_json,
                        "created_at": t.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(json!({ "transactions": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}
