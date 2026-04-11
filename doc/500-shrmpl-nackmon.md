# 500 - shrmpl-nackmon

> Negative acknowledgment monitor — watches for scheduled check-ins and alerts on misses.
> One binary. Env-based config. Cron-driven schedules. Slack alerts.

---

## Overview

**shrmpl-nackmon** monitors scheduled jobs by expecting periodic check-ins. If a job fails to check in within its grace period after a scheduled run, nackmon fires a Slack alert. If the job remains silent, escalation alerts follow at a configurable interval.

```
cron schedule fires → wait grace period → no check-in? → MISS alert → escalation alerts
                                          check-in?    → all clear
```

This is a "negative acknowledgment" model — silence is the signal. Jobs call a simple HTTP endpoint to say "I ran." If they don't, nackmon notices.

---

## Architecture

```
shrmpl-nackmon-srv <config.env>
       |
       ├── HTTP listener
       |
       ├── /health          → server health
       ├── /checkin?code=X  → job reports success
       ├── /ack?code=X      → operator silences alarm
       └── /<status-guid>   → monitor status (obscured path)
       |
       └── Background scheduler (ticks every 60s)
              └── For each monitor: evaluate cron → check grace → alert if missed
```

**Single argument**: path to the server config env file.

```bash
./shrmpl-nackmon-srv /etc/shrmpl/nackmon.env
```

---

## How It Works

### The Happy Path

1. A cron job runs at 02:00
2. Job completes and calls `GET /checkin?code=BKUP2024`
3. Nackmon records the check-in timestamp
4. Next scheduler tick sees the check-in arrived after the scheduled time — all clear

### The Alarm Path

1. A cron job is scheduled at 02:00 with `NACK_GRACE_MIN=30`
2. Job fails to run (or fails to call `/checkin`)
3. At 02:30, grace period expires — nackmon sends a **MISS** alert to Slack
4. Every `NACK_ESCALATION_GRACE_MIN` minutes thereafter, nackmon sends **ESCALATION** alerts
5. Alert continues until:
   - The job checks in (`/checkin?code=X`) — sends a "cleared" Slack message
   - An operator acknowledges (`/ack?code=X`) — sends a "silenced" Slack message

### Startup Behavior

On startup, nackmon does **not** alert for missed deadlines that occurred before the server started. This prevents duplicate alerts when restarting the service — the previous instance already handled those.

---

## Server Config

`nackmon.env` — the one argument passed to the binary.

```bash
# --- Network ---
NACK_LISTEN_ADDR=127.0.0.1
NACK_LISTEN_PORT=7575

# --- Identity ---
NACK_SERVER_NAME=prod-nackmon     # included in all Slack messages

# --- Paths ---
NACK_MONITORS_DIR=/etc/shrmpl/nack-monitors

# --- Status Endpoint (use a GUID to obscure) ---
NACK_STATUS_PATH=/status/b7e3f1a2-9c4d-4e8b-a1f0-2d3c4b5a6e7f

# --- Logging (shrmpl-log) ---
SLOG_DEST=10.0.0.5:7379
SLOG_LEVEL=INFO
SLOG_CONSOLE=true
SLOG_SEND_ACTV=true
SLOG_SEND_LOG=true
```

### Required fields

- `NACK_LISTEN_ADDR`, `NACK_LISTEN_PORT`
- `NACK_SERVER_NAME`
- `NACK_MONITORS_DIR`
- `NACK_STATUS_PATH`

---

## Monitor Env Files

Each monitored job is a `*.env` file in `NACK_MONITORS_DIR`:

```
nack-monitors/
├── nightly-backup-BKUP2024.env
├── log-rotation-LOGROT01.env
└── one-min-test-12345.env
```

File naming convention: `{name}-{CODE}.env`

- **name**: human-readable label (nightly-backup, log-rotation, etc.)
- **CODE**: unique check-in code used in the `/checkin` URL (4-25 characters)

On startup, the server scans `NACK_MONITORS_DIR` for all `*.env` files. It extracts the code (everything after the last `-` before `.env`) and the name (everything before).

### Monitor env format

```bash
# --- Schedule ---
# Cron format: MIN(0-59) HOUR(0-23) DOM(1-31) MON(1-12) DOW(0-6, 0=Sun)
NACK_CRON=0 2 * * *
NACK_DESCRIPTION=Database backup job
NACK_GRACE_MIN=30

# --- Alerting (optional — omit for log-only) ---
NACK_SLACK_WEBHOOK=https://hooks.slack.com/services/T.../B.../xxx
NACK_SLACK_MESSAGE=<@U05U7ENTJ1W>

# --- Escalation ---
NACK_ESCALATION_GRACE_MIN=60
```

### Required monitor fields

- `NACK_CRON` — standard 5-field cron expression
- `NACK_ESCALATION_GRACE_MIN` — minutes between escalation alerts

### Optional monitor fields

- `NACK_DESCRIPTION` — human label (defaults to the name from filename)
- `NACK_GRACE_MIN` — minutes to wait after scheduled time before alerting (default: 15)
- `NACK_SLACK_WEBHOOK` — Slack incoming webhook URL (omit for log-only)
- `NACK_SLACK_MESSAGE` — appended to all Slack messages (use for @-mentions)

---

## Slack Messages

All messages include `[server_name]`, job name, and code for identification.

### MISS (first alert)

```
:warning: [prod-nackmon] `nightly-backup` (code=BKUP2024) missed check-in. Database backup job <@U05U7ENTJ1W>
```

### ESCALATION (subsequent alerts)

```
:rotating_light: ESCALATION: [prod-nackmon] `nightly-backup` (code=BKUP2024) missed check-in (alert #3). Database backup job <@U05U7ENTJ1W>
```

### Cleared (job checked in)

```
:white_check_mark: [prod-nackmon] `nightly-backup` (code=BKUP2024) checked in — alarm cleared. <@U05U7ENTJ1W>
```

### Silenced (operator acknowledged)

```
:no_bell: [prod-nackmon] `nightly-backup` (code=BKUP2024) alarm silenced by operator. <@U05U7ENTJ1W>
```

---

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Server health check |
| GET | `/checkin?code=X` | Job reports successful completion |
| GET | `/ack?code=X` | Operator silences an active alarm |
| GET | `/{status-path}` | Monitor status (path set by NACK_STATUS_PATH) |

### GET /health

```json
{"status":"ok","monitors_loaded":3,"uptime_seconds":86400}
```

### GET /checkin?code=BKUP2024

```json
{"status":"ok"}
```

Returns 400 if code missing or invalid length. Returns 404 if code unknown.

If an alarm was active for this code, sends a "cleared" Slack notification and resets the alert counter.

### GET /ack?code=BKUP2024

```json
{"status":"acknowledged"}
```

or if no alarm is active:

```json
{"status":"no active alarm"}
```

Same validation as `/checkin`. Resets alert counter and sets last_checkin to now, preventing re-alerting until the next cron window passes without a check-in.

### GET /{status-path}

```json
[
  {
    "code": "BKUP2024",
    "name": "nightly-backup",
    "description": "Database backup job",
    "alert_count": 0,
    "last_checkin": "2026-04-07T02:15:00Z"
  }
]
```

The status path is configured via `NACK_STATUS_PATH` and should be set to a GUID to prevent casual discovery of monitored job status.

---

## Use Cases

### Monitoring a nightly backup

```bash
# Monitor config: nightly-backup-BKUP2024.env
NACK_CRON=0 2 * * *
NACK_GRACE_MIN=30
NACK_ESCALATION_GRACE_MIN=60
NACK_SLACK_WEBHOOK=https://hooks.slack.com/services/...
NACK_SLACK_MESSAGE=<@U05U7ENTJ1W>
```

In the backup script:
```bash
#!/bin/bash
pg_dump mydb > /backups/mydb.sql
curl -s "http://localhost:7575/checkin?code=BKUP2024"
```

### Silencing a false alarm after restart

```bash
# Nackmon restarted at 10:00 AM. Nightly backup ran at 2:00 AM.
# No alarm fires (deadline was before startup).
# But if nackmon had been running and alarming before restart:
curl "http://localhost:7575/ack?code=BKUP2024"
```

---

## Security Notes

- The status endpoint is obscured behind a GUID path to prevent enumeration
- Check-in codes are validated for length (4-25 chars) and matched against loaded monitors only
- Query string processing is capped at 2048 chars / 32 parameters
- No TLS — designed to run behind a reverse proxy or on a trusted internal network
- Slack webhook URLs in monitor env files are secrets — restrict file permissions (`chmod 600 *.env`)
