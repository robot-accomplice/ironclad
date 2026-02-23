# ironclad-schedule

> **Version 0.5.0**

Unified cron/heartbeat scheduler for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Provides DB-backed lease acquisition for single-instance job execution, a heartbeat daemon for periodic health checks and metric snapshots, and a durable scheduler with cron expression and interval evaluation.

## Key Types

| Type | Module | Description |
|------|--------|-------------|
| `HeartbeatDaemon` | `heartbeat` | Periodic tick loop driving registered heartbeat tasks |
| `TickContext` | `heartbeat` | Context passed to each tick (wallet, db references) |
| `DurableScheduler` | `scheduler` | Cron expression and interval evaluation |
| `HeartbeatTask` | `tasks` | Trait for pluggable heartbeat tasks |
| `TaskResult` | `tasks` | Success / skip / error outcome |

## Top-Level Functions

- `run_heartbeat()` -- Start the heartbeat daemon loop
- `run_cron_worker()` -- Start the cron worker loop (evaluate, lease, execute, record)

## Usage

```toml
[dependencies]
ironclad-schedule = "0.5"
```

```rust
use ironclad_schedule::{HeartbeatDaemon, run_heartbeat, run_cron_worker};

// Start heartbeat daemon (60s interval)
let daemon = HeartbeatDaemon::new(60_000);
tokio::spawn(async move {
    run_heartbeat(daemon, wallet, db).await;
});

// Start cron worker
tokio::spawn(async move {
    run_cron_worker(db, instance_id).await;
});
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-schedule).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
