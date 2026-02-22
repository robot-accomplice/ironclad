# ironclad-server

Autonomous agent runtime built in Rust as a single optimized binary. Features HTTP API (axum, 41 routes), embedded dashboard, CLI (24 commands), WebSocket push, and migration engine.

Part of the [Ironclad](https://github.com/robot-accomplice/ironclad) workspace.

## Installation

```bash
cargo install ironclad-server
```

## Usage

```bash
# Initialize configuration
ironclad init

# Start the server
ironclad serve

# Check system health
ironclad mechanic
```

## Documentation

- CLI help: `ironclad --help`
- API docs: [docs.rs](https://docs.rs/ironclad-server)
- Full documentation: [github.com/robot-accomplice/ironclad](https://github.com/robot-accomplice/ironclad)

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
