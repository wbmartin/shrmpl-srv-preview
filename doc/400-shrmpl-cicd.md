# 400 - shrmpl-cicd

> Lightweight webhook-driven CI/CD runner  
> One binary. Env-based config. No orchestrator required.

---

## Overview

**shrmpl-cicd** listens for incoming webhooks (GitHub, Azure DevOps, etc.) and executes a shell script defined in a per-hook env file. Each hook is identified by a GUID and configured with a `{prefix}-{guid}.env` file following existing shrmpl conventions.

```
webhook POST → match GUID → load env → run script → log output
```

---

## Architecture

```
shrmpl-cicd-srv <config.env>
       │
       ├── HTTPS listener (rustls, no mTLS)
       │        or
       └── HTTP listener (behind proxy)
       │
       ▼
  /hook/{guid}  ──→  hooks/{prefix}-{guid}.env
                         └── HOOK_SCRIPT=/opt/myapp/deploy.sh
```

**Single argument**: path to the server config env file.

```bash
./shrmpl-cicd-srv /etc/shrmpl/cicd.env
```

---

## Server Config

`cicd.env` — the one argument passed to the binary.

```bash
# --- Network ---
# TLS mode: "tls" or "plain" (pick one, not both)
CICD_TLS_MODE=tls

# Listen address
CICD_LISTEN_ADDR=0.0.0.0
CICD_LISTEN_PORT=8443

# TLS cert paths (ignored when CICD_TLS_MODE=plain)
CICD_TLS_CERT=/etc/shrmpl/cert.pem
CICD_TLS_KEY=/etc/shrmpl/key.pem

# --- Paths ---
# Directory containing hook env files
CICD_HOOKS_DIR=/etc/shrmpl/hooks

# Directory for run output logs
CICD_OUTPUT_DIR=/var/log/shrmpl-cicd

# --- Limits ---
CICD_MAX_CONCURRENT=4
CICD_DEFAULT_TIMEOUT=300

# --- Logging ---
CICD_LOG_LEVEL=info
```

### TLS Mode

Follows the shrmpl-vault pattern (server-side TLS only, no mTLS):

- `tls` — HTTPS with rustls. Requires `CICD_TLS_CERT` and `CICD_TLS_KEY`.
- `plain` — HTTP only. Use when behind a reverse proxy that terminates TLS.

Only one mode is active per instance. Not both.

---

## Hook Env Files

Each hook is a single env file in `CICD_HOOKS_DIR`:

```
hooks/
├── deploy-staging-a1b2c3d4.env
├── build-prod-e5f6g7h8.env
└── notify-test-i9j0k1l2.env
```

File naming convention: `{prefix}-{guid}.env`

- **prefix**: human-readable label (deploy-staging, build-prod, etc.)
- **guid**: the unique identifier used in the webhook URL

The webhook URL for a hook is: `POST /hook/{guid}`

On startup, the server scans `CICD_HOOKS_DIR` for all `*.env` files. It extracts the GUID from each filename (everything after the last `-` and before `.env`) and builds a routing table.

### hook.env format

```bash
# --- Identity ---
HOOK_NAME=deploy-staging
HOOK_DESCRIPTION=Deploy main branch to staging server

# --- Authentication ---
# Provider: github, azure-devops, generic
HOOK_PROVIDER=github

# Shared secret for webhook validation
# GitHub: used to verify X-Hub-Signature-256
# Azure DevOps: matched against basic auth or HTTP header
# generic: matched against X-Hook-Secret header
HOOK_SECRET=whsec_your_secret_here

# --- Execution ---
# Absolute path to the script to run
HOOK_SCRIPT=/opt/myapp/deploy.sh

# Working directory for script execution (defaults to script's parent dir)
HOOK_WORKING_DIR=/opt/myapp

# Timeout in seconds (overrides CICD_DEFAULT_TIMEOUT)
HOOK_TIMEOUT=600

# Stop execution on first non-zero exit code (applies to script's set -e)
HOOK_STOP_ON_FAIL=true

# --- Deduplication ---
# Number of recent delivery IDs to track (in-memory ring buffer)
HOOK_DEDUPE_WINDOW=50

# --- Optional filters ---
# Only trigger on specific branches (comma-separated, empty = all)
HOOK_BRANCH_FILTER=main,staging

# Only trigger on specific events (comma-separated, empty = all)
# GitHub: push, pull_request, release
# Azure DevOps: git.push, git.pullrequest.merged
HOOK_EVENT_FILTER=push
```

### The script

The script is a standalone bash script. It lives wherever makes sense — next to the app it deploys, in a shared scripts dir, wherever. shrmpl-cicd doesn't care. It just needs to be executable and at the path in `HOOK_SCRIPT`.

```bash
#!/usr/bin/env bash
set -euo pipefail

cd /opt/myapp
git pull origin main
./bin/build.sh
systemctl restart myapp

echo "Deploy complete at $(date)"
```

shrmpl-cicd injects these env vars into the script's environment:

```bash
SHRMPL_HOOK_GUID=a1b2c3d4
SHRMPL_DELIVERY_ID=abc123...
SHRMPL_TRIGGER_BRANCH=main
SHRMPL_TRIGGER_EVENT=push
SHRMPL_TRIGGER_REPO=org/repo
SHRMPL_TRIGGER_COMMIT=sha256...
SHRMPL_TRIGGER_TIMESTAMP=2026-02-22T10:30:00Z
```

All `HOOK_*` vars from the env file are also available to the script.

---

## Webhook Validation

### GitHub

Validates `X-Hub-Signature-256` header using HMAC-SHA256 with `HOOK_SECRET`.

Extracts from payload:
- Branch from `ref` field (strip `refs/heads/` prefix)
- Event from `X-GitHub-Event` header
- Delivery ID from `X-GitHub-Delivery` header

### Azure DevOps

Validates via HTTP header or basic auth. `HOOK_SECRET` format determines method:
- `basic:user:pass` — basic auth validation
- `header:value` — match against custom header value

Extracts from payload:
- Branch from `resource.refUpdates[].name` (strip `refs/heads/`)
- Event from `eventType` field
- Delivery ID from `id` field

### Generic

Matches `X-Hook-Secret` header against `HOOK_SECRET`. Delivery ID from `X-Delivery-ID` header, or generated from SHA256 of request body if header absent.

---

## Execution Model

1. **Receive** webhook POST at `/hook/{guid}`
2. **Lookup** — scan loaded hooks for matching GUID; 404 if not found
3. **Validate** — verify signature/secret per provider; 401 if invalid
4. **Dedupe** — check delivery ID against in-memory ring buffer; 429 if duplicate
5. **Filter** — check branch and event against filters; 422 if no match (not an error, just skipped)
6. **Lock** — if a run is already active for this GUID, reject with 409 Conflict
7. **Execute** — spawn `HOOK_SCRIPT` as child process via `tokio::process::Command`
8. **Stream** — capture stdout/stderr line-by-line to output file
9. **Complete** — record exit code, duration, update hook state to idle

### Concurrency

- One active run per GUID (no racing deploys)
- Global concurrency capped at `CICD_MAX_CONCURRENT`
- Excess requests get 503 Service Unavailable

### Output Logs

Run logs are written to `CICD_OUTPUT_DIR`, organized by GUID:

```
/var/log/shrmpl-cicd/
├── a1b2c3d4/
│   ├── 2026-02-22T10-30-00Z.log
│   └── 2026-02-22T14-15-00Z.log
└── e5f6g7h8/
    └── 2026-02-22T11-00-00Z.log
```

Log format:
```
[2026-02-22T10:30:00Z] START hook=deploy-staging guid=a1b2c3d4 delivery=abc123
[2026-02-22T10:30:00Z] BRANCH=main EVENT=push REPO=org/repo
[2026-02-22T10:30:01Z] [stdout] Already up to date.
[2026-02-22T10:30:05Z] [stdout] Build successful
[2026-02-22T10:30:07Z] [stdout] Deploy complete at Sat Feb 22 10:30:07 UTC 2026
[2026-02-22T10:30:07Z] END exit=0 duration=7s
```

---

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/hook/{guid}` | Receive webhook, trigger run |
| GET | `/status/{guid}` | Current status of a hook (idle/running/last result) |
| GET | `/health` | Server health check |

### POST /hook/{guid}

Response codes:
- **200** — accepted and running
- **401** — signature/secret validation failed
- **404** — unknown GUID
- **409** — run already in progress for this GUID
- **422** — filtered out (branch/event mismatch, not an error)
- **429** — duplicate delivery ID
- **503** — max concurrent runs reached

### GET /status/{guid}

```json
{
  "guid": "a1b2c3d4",
  "name": "deploy-staging",
  "state": "idle",
  "last_run": {
    "delivery_id": "abc123",
    "started_at": "2026-02-22T10:30:00Z",
    "finished_at": "2026-02-22T10:30:07Z",
    "exit_code": 0,
    "duration_seconds": 7
  }
}
```

### GET /health

```json
{
  "status": "ok",
  "hooks_loaded": 3,
  "active_runs": 1,
  "uptime_seconds": 86400
}
```

---

## Build & Run Scripts

Following the existing `bin/` numbering convention (4xx series):

```
bin/
├── 401-build-shrmpl-cicd-release    # cargo build --release (Linux target)
├── 405-run-shrmpl-cicd-dev          # cargo run with dev config
├── 410-test-shrmpl-cicd             # run tests
└── 415-test-webhook-github          # curl to simulate a GitHub webhook
```

### Development (macOS)

```bash
./bin/405-run-shrmpl-cicd-dev
# Runs with etc/shrmpl-cicd-srv-loc.env pointing to examples/hooks/
```

### Production (Linux)

```bash
./bin/401-build-shrmpl-cicd-release
# Output: dist/shrmpl-cicd-srv

cp dist/shrmpl-cicd-srv /usr/local/bin/
./shrmpl-cicd-srv /etc/shrmpl/cicd.env
```

---

## Example Configs

### etc/shrmpl-cicd-srv-loc.env

```bash
CICD_TLS_MODE=plain
CICD_LISTEN_ADDR=127.0.0.1
CICD_LISTEN_PORT=8080
CICD_HOOKS_DIR=./examples/hooks
CICD_OUTPUT_DIR=./tmp/cicd-output
CICD_MAX_CONCURRENT=2
CICD_DEFAULT_TIMEOUT=60
CICD_LOG_LEVEL=debug
```

### etc/cicd-prod.env

```bash
CICD_TLS_MODE=tls
CICD_LISTEN_ADDR=0.0.0.0
CICD_LISTEN_PORT=8443
CICD_TLS_CERT=/etc/shrmpl/cert.pem
CICD_TLS_KEY=/etc/shrmpl/key.pem
CICD_HOOKS_DIR=/etc/shrmpl/hooks
CICD_OUTPUT_DIR=/var/log/shrmpl-cicd
CICD_MAX_CONCURRENT=4
CICD_DEFAULT_TIMEOUT=300
CICD_LOG_LEVEL=info
```

### examples/hooks/echo-test-deadbeef.env

```bash
HOOK_NAME=echo-test
HOOK_DESCRIPTION=Test hook that echoes payload info
HOOK_PROVIDER=generic
HOOK_SECRET=test-secret-123
HOOK_SCRIPT=./examples/scripts/echo-test.sh
HOOK_WORKING_DIR=/tmp
HOOK_TIMEOUT=30
HOOK_STOP_ON_FAIL=true
HOOK_DEDUPE_WINDOW=10
HOOK_BRANCH_FILTER=
HOOK_EVENT_FILTER=
```

### examples/scripts/echo-test.sh

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "Hook triggered!"
echo "GUID: ${SHRMPL_HOOK_GUID}"
echo "Delivery: ${SHRMPL_DELIVERY_ID}"
echo "Branch: ${SHRMPL_TRIGGER_BRANCH:-none}"
echo "Event: ${SHRMPL_TRIGGER_EVENT:-none}"
echo "Done."
```

---

## Security Notes

- Hook env files contain secrets — restrict file permissions (`chmod 600 *.env`)
- GUID is validated against loaded hooks only; no filesystem path construction from user input
- Request body size should be capped (e.g., 1MB) to prevent abuse
- Rate limiting per GUID is handled implicitly by the one-run-at-a-time lock
- Consider running scripts as a dedicated user via `HOOK_RUN_AS` (future enhancement)

---

## Implementation Notes for Code Generation

These notes are intended for Claude Code or similar tools generating the Rust implementation. Since this doc author could not access all repo subdirectories, the code generator should inspect the existing source tree first and adapt these guidelines to match established patterns.

### Before You Start

1. Read the existing `Cargo.toml` — check workspace structure, dependency version pins, feature flags.
2. Read `src/` — understand how `shrmpl_kv_srv`, `shrmpl_log_srv`, `shrmpl_vault_srv` are structured (binary entry points, module layout, shared utilities).
3. Read `bin/` scripts — understand build, run, and test script conventions.
4. Read `etc/` — understand how existing components handle their config env files.
5. Read the shrmpl-vault TLS setup specifically — this is the model for shrmpl-cicd TLS (server-side only, no mTLS).
6. Match all conventions you find. This doc describes *what* to build; the existing code shows *how* to build it.

### Project Structure

This is a new binary target in the existing shrmpl workspace. Follow the existing `src/` layout. Suggested modules (adapt to match repo conventions):

```
src/
├── bin/
│   └── shrmpl_cicd_srv.rs    # main: parse arg, load config, start server
├── cicd/
│   ├── mod.rs
│   ├── config.rs              # parse cicd.env, parse hook.env files
│   ├── server.rs              # hyper server setup, TLS or plain
│   ├── router.rs              # route /hook/{guid}, /status/{guid}, /health
│   ├── webhook.rs             # provider-specific validation & payload parsing
│   │                          #   - github: HMAC-SHA256 validation
│   │                          #   - azure_devops: basic auth or header match
│   │                          #   - generic: header secret match
│   ├── executor.rs            # spawn script, capture output, manage timeouts
│   ├── state.rs               # shared state: hook registry, run locks, dedupe buffers
│   └── output.rs              # write run logs to CICD_OUTPUT_DIR
```

### Crate Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio"] }
http-body-util = "0.1"
rustls = "0.23"
tokio-rustls = "0.26"
rustls-pemfile = "2"
hmac = "0.12"
sha2 = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = { version = "1", features = ["v4"] }
```

Check existing `Cargo.toml` for version pinning conventions before adding these.

### Env File Parsing

Do not use a third-party `.env` crate unless the existing components already do. The existing shrmpl components likely parse env files manually. Follow that pattern:
- Read file line by line
- Skip empty lines and `#` comments
- Split on first `=`
- Trim whitespace and optional quotes from values
- Return a `HashMap<String, String>`

### Startup Sequence

1. Read single CLI arg (config path). Exit with usage message if missing.
2. Parse server config env file.
3. Validate required server config fields.
4. Scan `CICD_HOOKS_DIR` for `*.env` files.
5. For each file: extract GUID from filename, parse hook env, validate required fields (`HOOK_PROVIDER`, `HOOK_SECRET`, `HOOK_SCRIPT`).
6. Verify each `HOOK_SCRIPT` path exists and is executable. Warn (don't fail) if not.
7. Build shared state (`Arc<AppState>`) containing hook registry, run locks, dedupe buffers.
8. Create `CICD_OUTPUT_DIR` if it doesn't exist.
9. Start HTTP or HTTPS listener based on `CICD_TLS_MODE`.
10. Log startup summary: number of hooks loaded, listen address, TLS mode.

### GUID Extraction from Filename

Given filename `deploy-staging-a1b2c3d4.env`:
- Strip `.env` extension
- Split on `-`
- GUID is the last segment: `a1b2c3d4`
- Prefix is everything before: `deploy-staging`

GUIDs cannot contain hyphens. Use hex or alphanumeric only.

### Shared State

```rust
struct AppState {
    hooks: HashMap<String, HookConfig>,        // guid -> config
    run_locks: Mutex<HashSet<String>>,          // guids with active runs
    dedupe_buffers: Mutex<HashMap<String, VecDeque<String>>>,  // guid -> recent delivery IDs
    active_count: AtomicUsize,                  // global active run count
    config: ServerConfig,
    start_time: Instant,
    last_runs: Mutex<HashMap<String, LastRunInfo>>,  // guid -> last completed run
}
```

Use `tokio::sync::Mutex` for async-friendly locking. Concurrency is low enough that contention is not a concern.

### Request Handling Flow

The router should:
1. Match method + path segments
2. For `POST /hook/{guid}`: read full body into bytes (cap at 1MB), pass to handler
3. Handler: lookup → validate → dedupe → filter → lock → spawn → respond 200
4. The script execution happens in a `tokio::spawn` background task so the HTTP response returns immediately
5. Response body can include a brief JSON ack: `{"status": "accepted", "guid": "...", "delivery_id": "..."}`

### Script Execution

```rust
let mut cmd = tokio::process::Command::new("bash");
cmd.arg(&hook.script_path);
cmd.current_dir(&hook.working_dir);
cmd.envs(env_vars);  // merge HOOK_* vars + SHRMPL_* vars
cmd.stdout(Stdio::piped());
cmd.stderr(Stdio::piped());

let child = cmd.spawn()?;
```

Use `tokio::select!` with `tokio::time::sleep(Duration::from_secs(timeout))` for timeout enforcement. Kill the child process on timeout.

Read stdout/stderr concurrently using `BufReader::lines()` on each pipe. Interleave lines into the output log with `[stdout]` / `[stderr]` prefixes and timestamps.

After completion (or timeout), release the run lock and decrement `active_count`.

### TLS Setup

Follow the shrmpl-vault TLS pattern exactly:
- Load cert chain from PEM file using `rustls_pemfile`
- Load private key from PEM file
- Build `rustls::ServerConfig` with `no_client_auth()`
- Wrap TCP listener with `TokioRustlsAcceptor`

### Error Handling

- Config errors at startup: print to stderr and `std::process::exit(1)`
- Runtime errors (bad payload, script failure): log via `tracing`, return appropriate HTTP status
- Never panic on bad input from webhooks
- If a hook env file has invalid config, log a warning at startup and skip that hook (don't fail the whole server)

### Testing

Include at least:
- Unit tests for env file parsing (valid, comments, empty lines, quoted values)
- Unit tests for GUID extraction from filenames
- Unit tests for HMAC-SHA256 validation (use known GitHub test vectors)
- Unit tests for Azure DevOps basic auth parsing
- Unit tests for branch/event filter matching
- Integration test: start server on random port, POST a test webhook with valid signature, verify script ran and output log was written
- The `bin/415-test-webhook-github` script should compute a valid HMAC signature and curl the local dev server

### Cross-Compilation

Development on macOS, production on Linux. Check how existing `bin/101-*`, `bin/201-*`, `bin/301-*` build scripts handle this (likely one of: cross-compile with `--target x86_64-unknown-linux-gnu`, build on the Linux host, or use a Docker build container). Match the existing approach.

---

## Future Considerations (not in v1)

- shrmpl-log integration for centralized logging
- shrmpl-vault integration for secret retrieval at runtime
- `HOOK_RUN_AS` — run script as a specific OS user
- Retry logic for failed runs
- Webhook forwarding / chaining (run hook A then hook B)
- Web UI for status dashboard
- Notification callbacks (Slack, email) on success/failure
- Hot reload of hook env files without restart (watch `CICD_HOOKS_DIR`)
- Systemd service file and install script