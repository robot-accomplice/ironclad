# C4 Level 3: Component Diagram -- ironclad-schedule

*Unified scheduling system merging cron job execution and heartbeat daemon into a single tokio-based tick loop with DB-backed job state and lease-based concurrency control.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladSchedule ["ironclad-schedule"]
        HEARTBEAT["heartbeat.rs<br/>Heartbeat Daemon"]
        SCHEDULER["scheduler.rs<br/>Cron Scheduler"]
        TASKS["tasks.rs<br/>Built-in Heartbeat Tasks"]
    end

    subgraph HeartbeatDetail ["heartbeat.rs"]
        TICK_LOOP["Tick loop:<br/>tokio::time::interval<br/>(configurable, default 60s)<br/>select! pattern (no overlap)"]
        TICK_CTX["Build TickContext:<br/>- credit balance<br/>- USDC balance<br/>- SurvivalTier<br/>- current timestamp<br/>(all fetched once per tick)"]
        SLEEP_LOOP["Sleep loop:<br/>tokio::select!<br/>- mpsc::recv (wake event)<br/>- interval (30s poll fallback)"]
    end

    subgraph SchedulerDetail ["scheduler.rs"]
        QUERY_JOBS["query_due_jobs():<br/>SELECT FROM cron_jobs<br/>WHERE enabled = 1"]
        EVAL_CRON["evaluate_cron():<br/>parse cron expression<br/>(schedule_expr + schedule_tz)"]
        EVAL_INTERVAL["evaluate_interval():<br/>elapsed since last_run_at<br/>>= schedule_every_ms"]
        EVAL_AT["evaluate_at():<br/>now() >= schedule_expr<br/>(one-time fire)"]
        ACQUIRE["acquire_lease():<br/>UPDATE cron_jobs SET<br/>lease_holder = instance_id<br/>WHERE lease_holder IS NULL<br/>OR lease_expires_at < now()"]
        RELEASE["release_lease():<br/>UPDATE cron_jobs SET<br/>lease_holder = NULL"]
        NEXT_RUN["calculate_next_run():<br/>compute next fire time<br/>from schedule expression"]
    end

    subgraph TasksDetail ["tasks.rs"]
        SURVIVAL_CHECK["SurvivalCheck:<br/>monitor credit balance,<br/>compute SurvivalTier,<br/>adjust behavior tier"]
        USDC_MONITOR["UsdcMonitor:<br/>check on-chain USDC balance,<br/>signal wake if funds available"]
        YIELD_TASK["YieldTask:<br/>evaluate deposit/withdraw<br/>thresholds, execute via<br/>ironclad-wallet"]
        MEMORY_PRUNE["MemoryPrune:<br/>evict low-importance entries<br/>from episodic/working memory"]
        CACHE_EVICT["CacheEvict:<br/>prune expired semantic cache,<br/>LRU when over max_entries"]
        METRIC_SNAP["MetricSnapshot:<br/>record system metrics<br/>to metric_snapshots table"]
        AGENT_CARD_REFRESH["AgentCardRefresh:<br/>re-verify discovered_agents<br/>entries past TTL"]
    end

    subgraph Execution ["Job Execution"]
        PAYLOAD_KIND{"payload_json.kind?"}
        AGENT_TURN["agentTurn:<br/>inject message into agent loop<br/>(via tokio mpsc channel)"]
        SYS_EVENT["systemEvent:<br/>run system event handler<br/>(e.g., metric snapshot)"]
        SESSION_SELECT{"session_target?"}
        MAIN_SESSION["main: use existing session"]
        ISO_SESSION["isolated: create new session"]
    end

    subgraph Recording ["State Recording"]
        UPDATE_JOB["UPDATE cron_jobs:<br/>last_run_at, last_status,<br/>last_duration_ms, next_run_at,<br/>consecutive_errors, last_error,<br/>lease_holder = NULL"]
        INSERT_RUN["INSERT INTO cron_runs:<br/>job_id, status, duration_ms, error"]
    end

    subgraph Delivery ["Result Delivery"]
        DELIVER_MODE{"delivery_mode?"}
        SILENT["none: silent"]
        ANNOUNCE["announce: send via<br/>channel adapter<br/>(delivery_channel)"]
    end

    TICK_LOOP --> TICK_CTX --> QUERY_JOBS
    QUERY_JOBS --> EVAL_CRON & EVAL_INTERVAL & EVAL_AT
    EVAL_CRON & EVAL_INTERVAL & EVAL_AT --> ACQUIRE
    ACQUIRE --> PAYLOAD_KIND
    PAYLOAD_KIND --> AGENT_TURN & SYS_EVENT
    AGENT_TURN --> SESSION_SELECT
    SESSION_SELECT --> MAIN_SESSION & ISO_SESSION
```

## Wake Signal Flow

```mermaid
sequenceDiagram
    participant HB as heartbeat.rs
    participant Sched as scheduler.rs
    participant Task as tasks.rs (e.g., UsdcMonitor)
    participant MPSC as tokio mpsc channel
    participant Agent as ironclad-agent loop

    HB->>HB: tick (interval fires)
    HB->>Sched: evaluate all due jobs
    Sched->>Task: execute UsdcMonitor
    Task-->>Task: USDC balance > 0 detected
    Task->>MPSC: send WakeEvent
    MPSC->>Agent: recv WakeEvent
    Agent->>Agent: resume from sleep, process topup
```

## Dependencies

**External crates**: `tokio` (timers, mpsc, select!), `cron` (cron expression parsing)

**Internal crates**: `ironclad-core`, `ironclad-db`, `ironclad-agent`

**Depended on by**: `ironclad-server`
