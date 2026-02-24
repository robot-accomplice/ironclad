# Deployment Guide

This guide covers deploying Ironclad in production environments.

## Prerequisites

- **Rust toolchain** — 1.85+ (edition 2024)
- **SQLite** — 3.35+ (bundled via `rusqlite`, no system install required)
- **At least one LLM provider:**
  - Local: [Ollama](https://ollama.ai) (no API key needed)
  - Cloud: OpenAI, Anthropic, Google, OpenRouter, or Moonshot API key

## Quick Start

### Install from source

```bash
cargo install ironclad
```

### Install on Windows (PowerShell)

```powershell
irm https://roboticus.ai/install.ps1 | iex
```

### Initialize a workspace

```bash
ironclad init ~/.ironclad
cd ~/.ironclad
```

This creates an `ironclad.toml` and a `skills/` directory with starter skills.

### Run the setup wizard

```bash
ironclad setup
```

The wizard walks through provider selection, API key storage, model testing, and workspace configuration.

### Start the server

```bash
ironclad serve -c ironclad.toml
```

The dashboard is available at `http://127.0.0.1:18789`.

---

## Production Configuration

### Bind Address and API Key

For production deployments accessible beyond localhost, you **must** set an API key:

```toml
[server]
port = 18789
bind = "0.0.0.0"
api_key = "your-strong-secret-key"
```

Ironclad refuses to start on a non-localhost address without an API key.

### Persistent Database

Use a file-backed database instead of in-memory:

```toml
[database]
path = "~/.ironclad/state.db"
```

SQLite runs in WAL mode for concurrent read/write performance.

### Credential Storage

Store API keys securely using the encrypted keystore instead of environment variables:

```bash
ironclad keystore set OPENAI_API_KEY
# Enter value interactively (not stored in shell history)
```

Reference stored keys from config:

```toml
[providers.openai]
api_key_ref = "OPENAI_API_KEY"
```

### Security Audit

Run a security audit before deploying:

```bash
ironclad security audit -c ironclad.toml
```

This checks file permissions, API key presence, wallet encryption, bind address exposure, and sandbox configuration.

---

## Daemon Service

Ironclad can run as a persistent background service using launchd (macOS) or systemd (Linux). Native Windows service integration is not currently provided by `ironclad daemon`; on Windows, run `ironclad serve` directly or manage it with Task Scheduler/NSSM.

### Install as a Daemon

```bash
ironclad daemon install -c ~/.ironclad/ironclad.toml
```

On macOS, this creates a LaunchAgent plist at:
`~/Library/LaunchAgents/com.ironclad.agent.plist`

On Linux, this creates a systemd user service at:
`~/.config/systemd/user/ironclad.service`

### Manage the Daemon

```bash
ironclad daemon start
ironclad daemon stop
ironclad daemon restart
ironclad daemon status
```

### Uninstall

```bash
ironclad daemon uninstall
```

---

## Docker Deployment

### Dockerfile

```dockerfile
FROM rust:1.85-slim AS builder

WORKDIR /build
COPY . .
RUN cargo build --release --bin ironclad

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ironclad /usr/local/bin/ironclad

RUN useradd -m ironclad
USER ironclad

WORKDIR /home/ironclad/.ironclad
COPY --chown=ironclad:ironclad ironclad.toml .

EXPOSE 18789

ENTRYPOINT ["ironclad"]
CMD ["serve", "-c", "/home/ironclad/.ironclad/ironclad.toml"]
```

### Docker Compose

```yaml
services:
  ironclad:
    build: .
    ports:
      - "18789:18789"
    volumes:
      - ironclad-data:/home/ironclad/.ironclad
      - ./ironclad.toml:/home/ironclad/.ironclad/ironclad.toml:ro
    environment:
      - OPENAI_API_KEY=${OPENAI_API_KEY}
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:18789/api/health"]
      interval: 30s
      timeout: 5s
      retries: 3

volumes:
  ironclad-data:
```

### Build and run

```bash
docker compose up -d
```

### Docker config considerations

When running in Docker, set the bind address to `0.0.0.0` and use an API key:

```toml
[server]
bind = "0.0.0.0"
port = 18789
api_key = "your-secret-key"
```

---

## systemd Service (Manual)

If you prefer manual systemd setup over `ironclad daemon install`:

```ini
# ~/.config/systemd/user/ironclad.service
[Unit]
Description=Ironclad Autonomous Agent Runtime
After=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/ironclad serve -c %h/.ironclad/ironclad.toml
Restart=on-failure
RestartSec=5
Environment=HOME=%h

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable ironclad
systemctl --user start ironclad
systemctl --user status ironclad

# View logs
journalctl --user -u ironclad -f
```

---

## Reverse Proxy

### nginx

```nginx
upstream ironclad {
    server 127.0.0.1:18789;
}

server {
    listen 443 ssl http2;
    server_name agent.example.com;

    ssl_certificate     /etc/letsencrypt/live/agent.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/agent.example.com/privkey.pem;

    # WebSocket support for SSE streaming
    location / {
        proxy_pass http://ironclad;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # SSE streaming — disable buffering
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 86400s;
    }
}
```

### Caddy

```caddyfile
agent.example.com {
    reverse_proxy 127.0.0.1:18789
}
```

Caddy automatically provisions TLS certificates via Let's Encrypt.

---

## TLS Setup

### Let's Encrypt with Certbot

```bash
sudo apt install certbot python3-certbot-nginx
sudo certbot --nginx -d agent.example.com
```

### Self-signed (development only)

```bash
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem \
    -days 365 -nodes -subj "/CN=localhost"
```

Use a reverse proxy (nginx/Caddy) for TLS termination. Ironclad itself speaks plain HTTP.

---

## Database Backup & Restore

### Backup

SQLite databases can be safely copied while the server is running (WAL mode ensures consistency):

```bash
sqlite3 ~/.ironclad/state.db ".backup /backups/ironclad-$(date +%Y%m%d).db"
```

Or simply copy the file when the server is stopped:

```bash
cp ~/.ironclad/state.db /backups/ironclad-backup.db
```

### Automated backup script

```bash
#!/bin/bash
BACKUP_DIR="/backups/ironclad"
DB_PATH="$HOME/.ironclad/state.db"
DAYS_KEEP=30

mkdir -p "$BACKUP_DIR"
sqlite3 "$DB_PATH" ".backup $BACKUP_DIR/state-$(date +%Y%m%d-%H%M%S).db"
find "$BACKUP_DIR" -name "state-*.db" -mtime +$DAYS_KEEP -delete
```

Add to crontab for daily backups:

```bash
0 2 * * * /path/to/backup-ironclad.sh
```

### Restore

```bash
ironclad daemon stop
cp /backups/ironclad-backup.db ~/.ironclad/state.db
ironclad daemon start
```

---

## Log Management

Ironclad writes structured JSON logs to the configured `log_dir` (default: `~/.ironclad/logs/`).

### Log rotation

Logs are automatically retained for `log_max_days` days (default: 7). Configure in `ironclad.toml`:

```toml
[server]
log_dir = "~/.ironclad/logs"
log_max_days = 14
```

### View logs

```bash
# Recent logs
ironclad logs -n 100

# Follow in real-time
ironclad logs -f

# Filter by level
ironclad logs --level error

# API endpoint
curl http://localhost:18789/api/logs?lines=50&level=warn
```

### Log format

Each log line is a JSON object:

```json
{
  "timestamp": "2026-02-23T10:30:00.000Z",
  "level": "INFO",
  "fields": { "message": "Request processed" },
  "target": "ironclad_server::api"
}
```

### External log shipping

Pipe structured JSON logs to your preferred log aggregator:

```bash
tail -f ~/.ironclad/logs/ironclad.log | jq -c '.' | \
    your-log-shipper --format json
```

---

## Health Monitoring

### Health endpoint

```bash
curl -s http://localhost:18789/api/health | jq .
```

Returns `{"status": "ok", ...}` when healthy.

### Monitoring script

```bash
#!/bin/bash
HEALTH=$(curl -sf http://localhost:18789/api/health)
if [ $? -ne 0 ]; then
    echo "CRITICAL: Ironclad is not responding"
    # Send alert
    exit 2
fi

STATUS=$(echo "$HEALTH" | jq -r '.status')
if [ "$STATUS" != "ok" ]; then
    echo "WARNING: Ironclad status is $STATUS"
    exit 1
fi

echo "OK: Ironclad is healthy"
exit 0
```

### Metrics to monitor

- `GET /api/health` — Overall server health
- `GET /api/stats/costs` — Inference spending
- `GET /api/stats/cache` — Cache hit rate
- `GET /api/breaker/status` — Provider circuit breaker states
- `GET /api/agent/status` — Agent operational state

### Docker health check

```yaml
healthcheck:
  test: ["CMD", "curl", "-sf", "http://localhost:18789/api/health"]
  interval: 30s
  timeout: 5s
  retries: 3
  start_period: 10s
```

---

## Upgrade Procedures

### Check for updates

```bash
ironclad update check
```

### Update everything

```bash
ironclad update all --yes
```

This updates the binary, bundled provider configs, and the skills pack. Use `--no-restart` to prevent automatic daemon restart.

### Update components individually

```bash
# Binary only
ironclad update binary --yes

# Provider configurations
ironclad update providers --yes

# Skill pack
ironclad update skills --yes
```

### Manual upgrade steps

1. **Backup** the database and config:
   ```bash
   cp ~/.ironclad/state.db ~/.ironclad/state.db.bak
   cp ~/.ironclad/ironclad.toml ~/.ironclad/ironclad.toml.bak
   ```

2. **Stop** the server:
   ```bash
   ironclad daemon stop
   ```

3. **Update** the binary:
   ```bash
   cargo install ironclad
   ```

4. **Validate** the config:
   ```bash
   ironclad check -c ~/.ironclad/ironclad.toml
   ```

5. **Start** the server:
   ```bash
   ironclad daemon start
   ```

6. **Verify** health:
   ```bash
   curl -s http://localhost:18789/api/health | jq .
   ```

### Rollback

If an upgrade fails:

```bash
ironclad daemon stop
cp ~/.ironclad/state.db.bak ~/.ironclad/state.db
cp ~/.ironclad/ironclad.toml.bak ~/.ironclad/ironclad.toml
# Reinstall previous binary version
cargo install ironclad@0.4.2
ironclad daemon start
```

---

## Environment Sizing

### Minimum (single user, local models)

- **CPU:** 2 cores
- **RAM:** 4 GB (+ model requirements for Ollama)
- **Disk:** 1 GB for Ironclad, models vary

### Recommended (multi-channel, cloud models)

- **CPU:** 4 cores
- **RAM:** 8 GB
- **Disk:** 10 GB (database + logs + skills)

### Production (high availability)

- **CPU:** 8+ cores
- **RAM:** 16+ GB
- **Disk:** 50+ GB SSD
- External reverse proxy with TLS
- Automated backups
- Monitoring and alerting
