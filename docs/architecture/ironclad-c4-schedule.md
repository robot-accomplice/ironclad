# C4 Level 3: Component Diagram -- ironclad-schedule

*Heartbeat daemon (SurvivalTier-based interval adjustment) and durable cron worker. **run_heartbeat** and **run_cron_worker** in lib.rs; HeartbeatDaemon + TickContext in heartbeat.rs; DurableScheduler (cron/interval/at evaluation) in scheduler.rs; default_tasks() and HeartbeatTask enum in tasks.rs.*

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
        TICK_LOOP["run(): interval tick →<br/>build_tick_context() →<br/>execute default_tasks()"]
        TICK_CTX["TickContext: credit_balance,<br/>usdc_balance, survival_tier, timestamp"]
        ADJUST["should_adjust_interval(tier):<br/>LowCompute 2×, Critical 2×, Dead 10×<br/>cap: 5min non-dead, 1hr dead"]
        TICK_LOOP --> TICK_CTX
    end

    subgraph SchedulerDetail ["scheduler.rs"]
        QUERY_JOBS["query_due_jobs():<br/>SELECT cron_jobs WHERE enabled = 1"]
        EVAL["Schedule evaluation:<br/>· cron expr + timezone<br/>· interval (elapsed ≥ ms)<br/>· at (one-time fire)"]
        ACQUIRE["acquire_lease(): atomic UPDATE<br/>WHERE lease_holder IS NULL"]
        RELEASE["release_lease()"]
        NEXT_RUN["calculate_next_run()"]
        QUERY_JOBS --> EVAL --> ACQUIRE
    end

    subgraph TasksDetail ["tasks.rs"]
        direction LR
        TASK_ENUM["HeartbeatTask enum:<br/>SurvivalCheck, UsdcMonitor,<br/>YieldTask, MemoryPrune,<br/>CacheEvict, MetricSnapshot,<br/>AgentCardRefresh"]
    end

    subgraph Execution ["Job Execution"]
        PAYLOAD_KIND{"payload_json.kind?"}
        AGENT_TURN["agentTurn → inject message"]
        SYS_EVENT["systemEvent → handler"]
        SESSION_SELECT{"session_target?"}
        MAIN_SESSION["main session"]
        ISO_SESSION["isolated session"]
        PAYLOAD_KIND --> AGENT_TURN
        PAYLOAD_KIND --> SYS_EVENT
        AGENT_TURN --> SESSION_SELECT
        SESSION_SELECT --> MAIN_SESSION
        SESSION_SELECT --> ISO_SESSION
    end

    subgraph PostExec ["Post-Execution"]
        direction LR
        UPDATE_JOB["UPDATE cron_jobs<br/>(status, duration, next_run,<br/>lease = NULL)"]
        INSERT_RUN["INSERT cron_runs"]
        DELIVER_MODE{"delivery_mode?"}
        SILENT["silent"]
        ANNOUNCE["announce via channel"]
        UPDATE_JOB --> INSERT_RUN
        DELIVER_MODE --> SILENT
        DELIVER_MODE --> ANNOUNCE
    end

    TICK_CTX --> QUERY_JOBS
    ACQUIRE --> PAYLOAD_KIND
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

**External crates**: `tokio`, `chrono` (cron/interval/at time parsing). No separate cron crate — DurableScheduler uses chrono for expression evaluation.

**Internal crates**: `ironclad-core`, `ironclad-db`, `ironclad-agent`, `ironclad-wallet`

**Depended on by**: `ironclad-server`
