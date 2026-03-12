use ironclad_core::config::TreasuryConfig;
use ironclad_db::Database;
use ironclad_wallet::TreasuryPolicy;

#[test]
fn treasury_policy_with_db_transactions() {
    let db = Database::new(":memory:").unwrap();

    let policy = TreasuryPolicy::new(&TreasuryConfig {
        per_payment_cap: 100.0,
        hourly_transfer_limit: 500.0,
        daily_transfer_limit: 2000.0,
        minimum_reserve: 5.0,
        daily_inference_budget: 50.0,
        revenue_swap: Default::default(),
    });

    for i in 0..5 {
        ironclad_db::metrics::record_transaction(
            &db,
            "payment",
            80.0,
            "USDC",
            Some(&format!("vendor-{i}")),
            None,
        )
        .unwrap();
    }

    let recent = ironclad_db::metrics::query_transactions(&db, 1).unwrap();
    assert_eq!(recent.len(), 5);

    let hourly_total: f64 = recent.iter().map(|t| t.amount).sum();
    assert!((hourly_total - 400.0).abs() < f64::EPSILON);

    policy.check_hourly_limit(hourly_total, 100.0).unwrap();
    assert!(policy.check_hourly_limit(hourly_total, 100.01).is_err());

    policy.check_per_payment(100.0).unwrap();
    assert!(policy.check_per_payment(100.01).is_err());

    policy.check_minimum_reserve(100.0, 95.0).unwrap();
    assert!(policy.check_minimum_reserve(100.0, 95.01).is_err());
}

#[test]
fn inference_budget_tracking() {
    let db = Database::new(":memory:").unwrap();

    let policy = TreasuryPolicy::new(&TreasuryConfig {
        daily_inference_budget: 10.0,
        ..TreasuryConfig::default()
    });

    let mut total_cost = 0.0;
    for _ in 0..5 {
        let cost = 1.5;
        ironclad_db::metrics::record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            1000,
            500,
            cost,
            Some("T3"),
            false,
            None,
            None,
            false,
            None,
        )
        .unwrap();
        total_cost += cost;
    }

    assert!((total_cost - 7.5).abs() < f64::EPSILON);

    policy.check_inference_budget(total_cost, 2.5).unwrap();
    assert!(policy.check_inference_budget(total_cost, 2.51).is_err());
}

#[test]
fn check_all_validates_multiple_constraints() {
    let policy = TreasuryPolicy::new(&TreasuryConfig {
        per_payment_cap: 50.0,
        hourly_transfer_limit: 200.0,
        daily_transfer_limit: 1000.0,
        minimum_reserve: 10.0,
        daily_inference_budget: 50.0,
        revenue_swap: Default::default(),
    });

    policy.check_all(40.0, 100.0, 50.0, 200.0).unwrap();
    assert!(policy.check_all(60.0, 100.0, 0.0, 0.0).is_err());
    assert!(policy.check_all(40.0, 100.0, 170.0, 0.0).is_err());
    assert!(policy.check_all(40.0, 100.0, 0.0, 970.0).is_err());
    assert!(policy.check_all(40.0, 49.0, 0.0, 0.0).is_err());
}
