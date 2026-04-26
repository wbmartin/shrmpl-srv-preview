# 401 - shrmpl-cicd Tech Spec

> Single-file webhook runner. Receive, validate, execute, log.

---

## Scope

This is the implementation spec for `shrmpl-cicd-srv`, derived from [doc/400](400-shrmpl-cicd.md) with simplifications to match established project conventions.

### What changed from doc/400

- **Single file**: `src/shrmpl_cicd_srv.rs` — no submodules, matching all other srvs
- **shrmpl-log integration**: Uses `shrmpl_log_client::Logger` instead of `tracing`. No local output log files — all output goes through shrmpl-log
- **Dropped features**: branch/event filtering, HOOK_WORKING_DIR, HOOK_STOP_ON_FAIL, HOOK_NAME, HOOK_DESCRIPTION
- **Kept**: Delivery ID deduplication (prevents double-runs from provider retries)
- **Dependencies minimized**: No `tracing`, `tracing-subscriber`, `uuid` crates
- **All three providers kept**: GitHub, Azure DevOps, Generic (minimal extra code)
- **All three endpoints kept**: POST /hook/{guid}, GET /status/{guid}, GET /health

---

## File

```
src/shrmpl_cicd_srv.rs
```

Single binary target. ~400-600 lines estimated.

---

## Server Config

`cicd.env` — single CLI argument.

```bash
# --- Network ---
CICD_TLS_MODE=tls              # "tls" or "plain"
CICD_LISTEN_ADDR=0.0.0.0
CICD_LISTEN_PORT=8443
CICD_TLS_CERT=/etc/shrmpl/cert.pem
CICD_TLS_KEY=/etc/shrmpl/key.pem

# --- Paths ---
CICD_HOOKS_DIR=/etc/shrmpl/cicd-hooks

# --- Limits ---
CICD_MAX_CONCURRENT=4
CICD_DEFAULT_TIMEOUT=300

# --- Logging (shrmpl-log) ---
SLOG_DEST=10.0.0.5:5514        # shrmpl-log server address
SLOG_LEVEL=info                 # debug|info|warn|error
SLOG_CONSOLE=true               # also print to stdout
SLOG_SEND_ACTV=true             # send activity messages
SLOG_SEND_LOG=true              # send to remote log server
```

### Required fields

- `CICD_LISTEN_ADDR`, `CICD_LISTEN_PORT`
- `CICD_HOOKS_DIR`
- `CICD_MAX_CONCURRENT`, `CICD_DEFAULT_TIMEOUT`
- `SLOG_DEST`
- When `CICD_TLS_MODE=tls`: `CICD_TLS_CERT`, `CICD_TLS_KEY`

---

## Hook Env Files

Each hook is a `*.env` file in `CICD_HOOKS_DIR`:

```
hooks/
├── deploy-staging-a1b2c3d4.env
└── build-prod-e5f6g7h8.env
```

GUID extracted from filename: everything after the last `-` before `.env`.

### Hook env format

```bash
# --- Authentication ---
HOOK_PROVIDER=github            # github | azure-devops | generic
HOOK_SECRET=whsec_your_secret

# --- Execution ---
HOOK_SCRIPT=/opt/myapp/deploy.sh
HOOK_TIMEOUT=600                # optional, overrides CICD_DEFAULT_TIMEOUT

# --- Deduplication ---
HOOK_DEDUPE_WINDOW=50           # optional, default 50, ring buffer size
```

### Required hook fields

- `HOOK_PROVIDER`
- `HOOK_SECRET`
- `HOOK_SCRIPT`

---

## Webhook Validation

### GitHub

- Validate `X-Hub-Signature-256` header via HMAC-SHA256 of request body with `HOOK_SECRET`
- Extract: branch from `ref` (strip `refs/heads/`), event from `X-GitHub-Event`, delivery ID from `X-GitHub-Delivery`

### Azure DevOps

- `HOOK_SECRET` format determines validation method:
  - `basic:user:pass` — validate Authorization header (Basic auth)
  - `header:value` — match against `X-Hook-Secret` header
- Extract: branch from `resource.refUpdates[].name` (strip `refs/heads/`), event from `eventType`, delivery ID from `id`

### Generic

- Match `X-Hook-Secret` header against `HOOK_SECRET`
- Delivery ID from `X-Delivery-ID` header, or SHA256 of body if absent

---

## Injected Environment Variables

These are set in the script's environment at execution time:

```bash
SHRMPL_HOOK_GUID=a1b2c3d4
SHRMPL_DELIVERY_ID=abc123
SHRMPL_TRIGGER_BRANCH=main         # may be empty
SHRMPL_TRIGGER_EVENT=push          # may be empty
SHRMPL_TRIGGER_REPO=org/repo       # may be empty
SHRMPL_TRIGGER_COMMIT=sha256       # may be empty
SHRMPL_TRIGGER_TIMESTAMP=2026-02-22T10:30:00Z
```

All `HOOK_*` vars from the hook env file are also available.

---

## Endpoints

### POST /hook/{guid}

Receive webhook, validate, execute script.

Always responds **200** with a JSON ack if the request is well-formed. The caller is an automated service that can't act on errors, so we accept quickly and handle everything internally.

```json
{"status": "accepted", "guid": "a1b2c3d4", "delivery_id": "abc123"}
```

Rejection cases (validation failure, duplicate, already running, max concurrent) are logged via shrmpl-log but still return 200. The only non-200 responses:
- **404** — unknown GUID (so misconfigured webhooks are visible in the provider's delivery log)
- **413** — body too large

### GET /status/{guid}

```json
{
  "guid": "a1b2c3d4",
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

Returns 404 if GUID unknown. `last_run` is null if never run.

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

## Execution Flow

1. **Receive** POST at `/hook/{guid}`
2. **Lookup** GUID in loaded hooks → 404
3. **Validate** signature/secret per provider → 401
4. **Dedupe** delivery ID against ring buffer → 429
5. **Check** global active count < `CICD_MAX_CONCURRENT` → 503
6. **Lock** GUID (one run at a time) → 409
7. **Respond** 200 immediately
8. **Spawn** script in background `tokio::spawn`:
   - `tokio::process::Command::new(&script_path)` (honors shebang — bash, python, whatever)
   - Set env vars (SHRMPL_* + HOOK_*)
   - Pipe stdout/stderr
   - Stream lines through shrmpl-log with `[stdout]`/`[stderr]` prefix
   - Enforce timeout via `tokio::select!` + `tokio::time::sleep`
   - On complete/timeout: log result, release lock, decrement active count, update last_run

---

## Shared State

```rust
struct AppState {
    hooks: HashMap<String, HookConfig>,
    run_locks: tokio::sync::Mutex<HashSet<String>>,
    dedupe_buffers: tokio::sync::Mutex<HashMap<String, VecDeque<String>>>,
    active_count: AtomicUsize,
    config: ServerConfig,
    start_time: Instant,
    last_runs: tokio::sync::Mutex<HashMap<String, LastRunInfo>>,
    logger: shrmpl_log_client::Logger,
}
```

Wrapped in `Arc<AppState>`.

---

## Log Codes

Following the existing 3-4 char uppercase convention:

| Code | Level | When |
|------|-------|------|
| CICDSTART | info | Server startup with listen addr, TLS mode, hook count |
| CICDHOOK | info | Hook loaded at startup |
| CICDRECV | info | Webhook received |
| CICDRUN | info | Script execution started |
| CICDDONE | info | Script completed (includes exit code, duration) |
| CICDOUT | debug | Script stdout/stderr line |
| CICDFAIL | error | Script failed or timed out |
| CICDAUTH | warn | Validation failed |
| CICDDUP | debug | Duplicate delivery rejected |
| CICDLOCK | warn | Run already in progress, rejected |
| CICDLIMIT | warn | Max concurrent reached, rejected |
| CICDSHUT | info | Shutdown signal received |

---

## Dependencies

Only add what's not already in `Cargo.toml`:

```toml
hmac = "0.12"
sha2 = "0.10"
```

Everything else (`tokio`, `hyper`, `hyper-util`, `http-body-util`, `rustls`, `tokio-rustls`, `rustls-pemfile`, `serde_json`, `chrono`) should already be present from existing srvs. Verify before adding.

---

## Startup Sequence

1. Print version
2. Parse single CLI arg (config path) or exit with usage
3. Load config via `shrmpl::config::load_config()`
4. Initialize `shrmpl_log_client::Logger`
5. Scan `CICD_HOOKS_DIR` for `*.env` files
6. For each: extract GUID, parse config, validate required fields, warn if `HOOK_SCRIPT` not found
7. Build `Arc<AppState>`
8. Start TLS or plain listener (match vault-srv pattern)
9. Log startup summary via logger
10. Listen with graceful shutdown on SIGINT/SIGTERM

---

## Error Handling

- **Startup**: fail fast with `expect()` on required config. Skip individual hooks with bad config (log warning, don't crash).
- **Runtime**: never panic on webhook input. Log and return HTTP status codes.
- **Script errors**: log exit code and stderr via shrmpl-log. Not a server error.

---

## Build & Run Scripts

```
bin/401-build-shrmpl-cicd-release   # cargo build --release
bin/405-run-shrmpl-cicd-dev         # cargo run with etc/shrmpl-cicd-srv-loc.env
bin/415-test-webhook-github         # curl with computed HMAC signature
```

### Config files

```
etc/shrmpl-cicd-srv-loc.env                    # local dev (plain HTTP, localhost)
etc/cicd-prod.env                   # production template
etc/cicd-hooks/echo-test-deadbeef.env
examples/scripts/echo-test.sh
```

---

## Request Body Size

Cap at 1MB. Reject with 413 if exceeded.

---

## Security

- Hook env files contain secrets — document `chmod 600 *.env`
- GUID validated against in-memory map only, never used in filesystem paths from request
- Request body capped at 1MB
- One-run-at-a-time per GUID prevents race conditions
