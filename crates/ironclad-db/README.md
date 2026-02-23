# ironclad-db

> **Version 0.5.0**

SQLite persistence layer for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Provides typed CRUD operations over a unified database with WAL mode, FTS5 full-text search, BLOB-optimized embedding storage, HNSW ANN index, memory consolidation, session checkpointing, and semantic cache persistence.

## Key Types & Modules

| Module | Description |
|--------|-------------|
| `schema` | Table definitions, migration runner, connection management |
| `sessions` | Session and message CRUD, turn persistence |
| `memory` | 5-tier memory system (working, episodic, semantic, procedural, relationship) |
| `embeddings` | BLOB embedding storage with JSON fallback |
| `ann` | HNSW approximate nearest-neighbor index (instant-distance) |
| `hippocampus` | Long-term memory consolidation and decay |
| `checkpoint` | Session checkpoint and restore via `context_snapshots` |
| `efficiency` | Efficiency metrics tracking |
| `agents` | Sub-agent registry and enabled-agent CRUD |
| `backend` | Storage backend abstraction |
| `cache` | Semantic cache persistence (loaded on boot, flushed periodically) |
| `cron` | Cron job state, leases, run history |
| `skills` | Skill definition CRUD and trigger lookup |
| `tools` | Tool call records |
| `policy` | Policy decision records |
| `metrics` | Inference cost tracking, transactions, turn feedback |

## Usage

```toml
[dependencies]
ironclad-db = "0.5"
```

```rust
use ironclad_db::Database;

let db = Database::new(":memory:")?;
// Database is initialized with WAL mode, foreign keys, and all tables
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-db).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
