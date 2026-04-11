# 501 - shrmpl-nackmon Tech Spec

> Single-file negative ack monitor. Cron schedules, grace periods, Slack escalation.

---

## Scope

This is the implementation spec for `shrmpl-nackmon-srv`, derived from [doc/500](500-shrmpl-nackmon.md).

### Design decisions

- **Single file**: `src/shrmpl_nackmon_srv.rs` — no submodules, matching all other srvs
- **shrmpl-log integration**: Uses `shrmpl_log_client::Logger`
- **Plain HTTP only**: No TLS — runs on internal network or behind proxy
- **Cron via `croner` crate**: Standard 5-field cron parsing
- **Slack via HTTPS**: Uses `hyper-rustls` for outbound webhook calls

---

## File

```
src/shrmpl_nackmon_srv.rs
```

Single binary target. ~500 lines.

---

## Data Types

```rust
struct ServerConfig {
    listen_addr: String,
    listen_port: u16,
    monitors_dir: String,
    status_path: String,
    server_name: String,
}

struct MonitorConfig {
    code: String,
    name: String,
    description: String,
    cron_expr: String,
    grace_min: u64,
    slack_webhook: Option<String>,
    slack_message: Option<String>,
    escalation_grace_min: u64,
}

struct MonitorState {
    alert_count: u32,
    last_checkin: Option<DateTime<Utc>>,
    last_alert: Option<DateTime<Utc>>,
}

struct AppState {
    config: ServerConfig,
    monitors: HashMap<String, MonitorConfig>,
    monitor_states: Mutex<HashMap<String, MonitorState>>,
    start_time: DateTime<Utc>,
    logger: Logger,
}
```

Wrapped in `Arc<AppState>`. Uses `tokio::sync::Mutex`.

---

## Monitor File Loading

Filename convention: `{name}-{CODE}.env`

```
nightly-backup-BKUP2024.env
  name = "nightly-backup"
  code = "BKUP2024"
```

Extraction: strip `.env`, split on last `-`. Name is the prefix, code is the suffix.

### Required fields

- `NACK_CRON` — 5-field cron expression, validated with `croner::Cron`
- `NACK_ESCALATION_GRACE_MIN` — must parse as u64

### Optional fields with defaults

- `NACK_DESCRIPTION` — defaults to name from filename
- `NACK_GRACE_MIN` — defaults to 15
- `NACK_SLACK_WEBHOOK` — omit for log-only alerting
- `NACK_SLACK_MESSAGE` — appended to Slack messages (for @-mentions)

Monitors with missing required fields are skipped with a warning at startup.

---

## Scheduler

Background task, ticks every 60 seconds.

### Per-monitor evaluation on each tick

```
1. Parse cron expression
2. Find most recent cron match whose grace period has expired:
   last_scheduled = find_last_scheduled(cron, now - 24h, now - grace_min)
3. If none found → skip (no expired deadline)
4. If deadline (last_scheduled + grace_min) < start_time → skip (pre-startup)
5. If last_checkin >= last_scheduled → checkin OK, reset alert_count
6. Otherwise → miss detected:
   a. First alert (last_alert is None): always fire
   b. Subsequent: fire if now - last_alert >= escalation_grace_min
   c. Increment alert_count, set last_alert = now
   d. alert_count == 1 → MISS, alert_count > 1 → ESCALATION
   e. Send Slack if webhook configured
```

### find_last_scheduled

Iterates cron matches forward from `start` to `end`, returns the last one before `end`. Used with `end = now - grace_min` to ensure only expired deadlines are considered.

---

## Endpoints

### Request routing

```rust
match (&method, path) {
    (GET, "/health")   → handle_health
    (GET, "/checkin")  → handle_checkin
    (GET, "/ack")      → handle_ack
    (GET, status_path) → handle_status
    _                  → 404
}
```

### Query parameter parsing

Shared `parse_code_param()` function used by `/checkin` and `/ack`:
- Cap query string at 2048 chars
- Parse up to 32 `key=value` pairs
- Extract `code` parameter
- Validate length: 4-25 characters
- Return 400 if missing or invalid

### /checkin behavior

1. Parse and validate code
2. Lookup monitor → 404 if unknown
3. Record `last_checkin = now`, reset `alert_count = 0`
4. If alarm was active (`alert_count > 0` before reset):
   - Send Slack "cleared" notification
5. Return `{"status":"ok"}`

### /ack behavior

1. Parse and validate code
2. Lookup monitor → 404 if unknown
3. Set `last_checkin = now`, reset `alert_count = 0`
4. If alarm was active:
   - Send Slack "silenced" notification
   - Return `{"status":"acknowledged"}`
5. If no alarm was active:
   - Return `{"status":"no active alarm"}`

### /status behavior

Returns JSON array of all monitors with current state. Path is configurable via `NACK_STATUS_PATH` (should be set to a GUID for obscurity).

---

## Slack Notifications

Outbound HTTPS POST to Slack incoming webhook URL.

### Message format

All messages include `[server_name]`, backtick-highlighted job name, `(code=X)`, and the optional `NACK_SLACK_MESSAGE` suffix.

| Event | Icon | Format |
|-------|------|--------|
| First miss | `:warning:` | `[srv] \`name\` (code=X) missed check-in. description suffix` |
| Escalation | `:rotating_light:` | `ESCALATION: [srv] \`name\` (code=X) missed check-in (alert #N). description suffix` |
| Cleared | `:white_check_mark:` | `[srv] \`name\` (code=X) checked in — alarm cleared. suffix` |
| Silenced | `:no_bell:` | `[srv] \`name\` (code=X) alarm silenced by operator. suffix` |

### Implementation

- Uses `hyper::Client` with `hyper-rustls` HTTPS connector
- 10-second timeout on Slack requests
- Failures logged but never crash the server

---

## Log Codes

| Code | Level | When |
|------|-------|------|
| NACKMISS | warn | First missed check-in |
| NACKESCL | warn | Escalation alert |
| NACKCHKIN | info | Check-in received |
| NACKACK | info | Alarm acknowledged |
| NACKCRON | error | Bad cron expression |
| NACKSLAK | error/warn | Slack delivery failure |
| NACKLOAD | info | Monitor loaded at startup |
| NACKHTTP | info | HTTP server started |

---

## Startup Sequence

1. Print version
2. Parse single CLI arg (config path) or exit with usage
3. Load config via `shrmpl::config::load_config()`
4. Extract required server config fields (expect/panic if missing)
5. Initialize `shrmpl_log_client::Logger`
6. Check SLOG connectivity
7. Scan `NACK_MONITORS_DIR` for `*.env` files
8. For each: extract code from filename, parse config, validate required fields
9. Build `Arc<AppState>` with `start_time = Utc::now()`
10. Bind TCP listener
11. Spawn background scheduler task
12. Start HTTP server with graceful shutdown on Ctrl+C

---

## Dependencies

Only `croner = "2"` was added for this component. All other dependencies (`tokio`, `hyper`, `chrono`, `hyper-rustls`) were already present.

---

## Error Handling

- **Startup**: fail fast with `expect()` on required server config. Skip individual monitors with bad config (log warning, don't crash).
- **Runtime**: never panic on HTTP input. Validate query params, return appropriate status codes.
- **Slack failures**: log and continue. Never block the scheduler on a failed webhook call.

---

## Security

- Status endpoint obscured behind configurable GUID path
- Query string capped at 2048 chars / 32 params
- Check-in codes validated for length (4-25 chars)
- No filesystem path construction from user input
- Slack webhook URLs are secrets — env files should be `chmod 600`
