# Go Rewrite Spec

## Context

This document is the authoritative spec for rewriting the shrmpl service stack from Rust into Go in a new repository. It captures architectural decisions made after a full review of the existing codebase. A new Claude session should be able to use this document to plan and execute the rewrite without access to the prior conversation.

The existing Rust repo (`shrmpl-srv-preview`) remains the reference implementation during the transition. Do not modify it.

## Instructions for the Implementing Agent

Before writing any code for a service:

1. **Read the corresponding Rust source file in full.** The spec captures intent and key decisions but is not exhaustive — the Rust implementation is the ground truth for behavior, edge cases, and protocol details.
2. **Cross-reference the spec against the source.** If the source contains behavior, commands, endpoints, or config vars not mentioned in the spec, add them to the spec before implementing. Do not silently skip them.
3. **If something in the Rust source is unclear or ambiguous, stop and ask** rather than guessing. Correctness decisions made in the Rust version were intentional — assume there is a reason before changing anything.
4. **Implement in this order:** `internal/config` → `internal/logging` → `shrmpl-kv-srv` → `shrmpl-cicd-srv` → read and specify vault/nackmon/pulsecheck → implement those three. The shared packages must be complete before any service is started.
5. **Do not port the shrmpl-log client.** Any reference to `shrmpl_log_client` in the Rust source is replaced by `internal/logging` in Go. Do not replicate the TCP logging behavior.

---

## Philosophy

These are the non-negotiable design principles that the rewrite must preserve:

- **Simple binaries.** Each service compiles to a single static binary. No Docker required to run locally or in production. The same binary that runs in dev runs on the server.
- **Env file config.** Each service is configured via a `.env`-style file (KEY=VALUE, one per line, `#` comments). The config file path is passed as a CLI argument. Per-hook or per-endpoint config files follow the same format. No YAML, no TOML.
- **Own the stack.** No cloud-native dependencies, no vendor-managed services. Everything runs on a VM under direct operator control.
- **Minimal surface.** Resist feature creep. Each service does one thing. If a feature isn't needed now, don't build it.
- **Correctness over cleverness.** The Rust implementation reached correct behavior in concurrency, auth validation, and error handling. The Go rewrite must preserve that correctness — don't simplify away safety properties.

---

## What Is Being Deprecated

### shrmpl-log (do not port)

The custom TCP log aggregation server (`shrmpl-log-srv`) and its client library (`shrmpl-log-client`) are being removed entirely. Every service currently imports the log client and opens a new TCP connection per log call — this is the primary performance and reliability concern.

**Replacement:** `log/slog` (Go standard library, added in Go 1.21) with the text handler. Output goes to stdout/stderr and is captured by systemd journal automatically. No log server, no network dependency.

**Log format (text handler):**

```
time=2026-04-26T12:04:11Z level=INFO msg="Server started" addr=0.0.0.0:6379 code=KVSTART
```

Single line, human readable, structured key=value, greppable.

**Source location:** Every log line must include the source file and line number. Enable via `AddSource: true` in the slog handler options. This adds a `source=` field to every line:

```
time=2026-04-26T12:04:11Z level=INFO source=kv_srv.go:142 msg="Server started" addr=0.0.0.0:6379 code=KVSTART
```

**Timestamp toggle:** When running under systemd, journald adds its own timestamps. Services must support a `LOG_JOURNAL=true` env var that suppresses the `time=` field from slog output to avoid duplication. Implement this as a custom slog handler or handler options at startup.

**Preserving the CODE concept:** The existing codebase uses 12-char event codes (e.g., `KVSTART`, `VAULTACCESS`, `CICDRECV`) that make logs machine-searchable. Preserve these as a structured slog attribute:

```go
slog.Info("Server started", "code", "KVSTART", "addr", addr)
```

**Preserving the ACTV level:** The existing system has an `ACTV` severity for audit/business events distinct from INFO. In Go, implement this as a custom slog level (slog supports custom numeric levels) or as a structured field `kind=activity`. The goal is that audit events are distinguishable from operational noise in log output.

---

## New Repository

**Repo name:** shrmpl

**Go module structure:**

```
cmd/
  shrmpl-kv-srv/
  shrmpl-vault-srv/
  shrmpl-cicd-srv/
  shrmpl-nackmon-srv/
  shrmpl-pulsecheck-srv/
internal/
  config/       # env file loader
  logging/      # slog wrapper with CODE, ACTV, LOG_JOURNAL support
```

Build all binaries: `go build ./cmd/...`

**CLI convention:** Every service takes exactly one argument — the path to its config file. No flags, no subcommands.

```
shrmpl-kv-srv /etc/shrmpl/kv.env
shrmpl-vault-srv /etc/shrmpl/vault.env
shrmpl-cicd-srv /etc/shrmpl/cicd.env
shrmpl-nackmon-srv /etc/shrmpl/nackmon.env
shrmpl-pulsecheck-srv /etc/shrmpl/pulsecheck.env
```

If the argument is missing or the file cannot be read, the service must print usage and exit with a non-zero code.

Use the latest stable Go release (1.24 at time of writing). Minimize external dependencies — prefer standard library. External dependencies require justification.

---

## Service Specs

### shrmpl-kv-srv

**What it does:** In-memory key-value store. Volatile by design — all data is lost on process restart. No persistence layer. Multiple application servers can share state through a central instance.

**Protocol:** Line-oriented TCP. Client sends a command line, server responds with a result line. Each connection handles one command and closes (stateless).

**Commands to implement:**

| Command | Syntax                           | Notes                                                                      |
| ------- | -------------------------------- | -------------------------------------------------------------------------- |
| PING    | `PING`                           | Returns `PONG`                                                             |
| GET     | `GET <key>`                      | Returns value or `*KEY NOT FOUND*`                                         |
| SET     | `SET <key> <value> [expiration]` | Value auto-typed as int or string. Optional TTL.                           |
| INCR    | `INCR <key> [expiration]`        | Increments integer value. Creates at 1 if missing. Preserves existing TTL. |
| DEL     | `DEL <key>`                      | Returns `OK` or `*KEY NOT FOUND*`                                          |
| LIST    | `LIST`                           | Returns all keys, values, and expiration timestamps                        |
| BATCH   | `BATCH <cmd1>;<cmd2>;<cmd3>`     | Runs up to 3 semicolon-delimited commands, returns semicolon-delimited results |

**BATCH details:** Commands are split on `;` and each is processed as a normal command. Results are joined with `;` and returned on a single line. Maximum 3 commands — returns `ERROR too many commands` if exceeded. Empty segments between semicolons are ignored.

**Expiration format:** The existing implementation uses a `parse_expiration` function — check the Rust source for the accepted formats (e.g., `30s`, `5m`, `1h`).

**Concurrency:** Use a `sync.RWMutex`-protected map. Read operations (GET, LIST) take a read lock. Write operations (SET, INCR, DEL) take a write lock. Expired keys should be treated as not found on read and cleaned up lazily.

**Key/value constraints:** Max key length 100 bytes, max value length 100 bytes.

**Config vars:**

- `KV_BIND_ADDR`
- `KV_BIND_PORT`
- `SLOG_*` (see logging section)

---

### shrmpl-vault-srv

**What it does:** Secrets/config server. Services retrieve their configuration from vault at startup rather than storing secrets on their local filesystem. Access control is via mTLS — a service must present a valid client certificate to connect.

**Architecture:** Services hold a client certificate. At startup they connect to vault, authenticate via the cert, and retrieve their config as key-value pairs. The vault server holds the secrets; app servers hold only certs.

**Why this matters:** This is better than encrypted files on the app server — a compromised app server gives an attacker the cert but not the secrets (vault can be on a separate host and access can be revoked).

**Protocol:** mTLS over TCP or HTTPS. Preserve the existing protocol exactly — check `src/shrmpl_vault_srv.rs` in the Rust repo for the wire format.

**Open item:** Cert issuance and rotation is the hard part of this architecture. The rewrite should document the cert lifecycle (how new service certs are provisioned, how rotation is triggered) even if the implementation is manual. This was not fully resolved in the Rust version.

**Config vars:**

- `VAULT_BIND_ADDR`
- `VAULT_BIND_PORT`
- `VAULT_TLS_CERT` — server cert path
- `VAULT_TLS_KEY` — server key path
- `VAULT_CA_CERT` — CA cert for validating client certs
- `VAULT_DATA_DIR` — where secret env files are stored
- `SLOG_*`

---

### shrmpl-cicd-srv

**What it does:** Webhook receiver that triggers build scripts. Receives a webhook from a repo host, validates the signature, and runs a configured shell script. Notifies via Slack on start and completion.

**Endpoints:**

- `POST /hook/<guid>` — receive webhook
- `GET /status/<guid>` — current state and last run info for a hook
- `GET /health` — server health, hook count, active runs, uptime

**Hook config:** Each hook is a `.env` file in the hooks directory. Filename format: `<name>-<guid>.env`. The GUID is extracted by splitting on the **last** hyphen in the filename stem — meaning the name portion may contain hyphens but the GUID must not. Example: `deploy-api-srv-abc123.env` → name=`deploy-api-srv`, guid=`abc123`. A filename with no hyphen is skipped with a warning. Config vars per hook:

- `HOOK_PROVIDER` — `github`, `azure-devops`, or `generic`
- `HOOK_SECRET` — HMAC secret (GitHub), `basic:user:pass` or `header:value` (Azure DevOps), or token (generic)
- `HOOK_SCRIPT` — absolute path to script to execute
- `HOOK_TIMEOUT` — seconds before killing the script
- `HOOK_DEDUPE_WINDOW` — number of recent delivery IDs to remember for deduplication
- `HOOK_SLACK_WEBHOOK` — Slack webhook URL (optional)
- Any other vars are injected into the script environment as-is

**Webhook validation — preserve exactly:**

- **GitHub:** HMAC-SHA256 of request body using `HOOK_SECRET`, compared against `X-Hub-Signature-256` header. Constant-time comparison required.
- **Azure DevOps:** Either `basic:user:pass` (validates `Authorization: Basic <b64>`) or `header:value` (validates `X-Hook-Secret` header). Constant-time comparison required.
- **Generic:** Validates `X-Hook-Secret` header against secret. Constant-time comparison required.

**Script execution:**

- Script runs with env vars injected: `SHRMPL_HOOK_GUID`, `SHRMPL_DELIVERY_ID`, `SHRMPL_TRIGGER_BRANCH`, `SHRMPL_TRIGGER_EVENT`, `SHRMPL_TRIGGER_REPO`, `SHRMPL_TRIGGER_COMMIT`, `SHRMPL_TRIGGER_TIMESTAMP`
- All `HOOK_*` vars from the hook config file are also injected
- stdout and stderr from the script are logged at DEBUG level
- Timeout kills the process and records exit code -1

**Concurrency controls:**

- **Global work queue:** Incoming webhook requests are accepted immediately (HTTP 200) and placed in a FIFO queue. A worker pool (size = `CICD_MAX_CONCURRENT`) pulls from the queue and executes jobs. Queue capacity is 5 pending jobs — if the queue is full, the request is rejected with a `queue_full` reason. This replaces the Rust per-hook run lock + reject-when-busy model.
- **Per-hook deduplication:** Sliding window of recent delivery IDs checked before enqueuing. Duplicates are rejected before they enter the queue.
- **Queue depth in status:** `GET /health` should report current queue depth so operators can see backpressure.

**TLS:** None. cicd runs plain HTTP and is expected to sit behind an nginx reverse proxy which handles TLS termination. Webhook auth security is provided by payload signature validation, not transport encryption.

**Server config vars:**

- `CICD_LISTEN_ADDR`, `CICD_LISTEN_PORT`
- `CICD_HOOKS_DIR`
- `CICD_MAX_CONCURRENT`
- `CICD_DEFAULT_TIMEOUT`
- `CICD_SERVER_NAME` — used in Slack messages
- `SLOG_*`

---

### shrmpl-nackmon-srv

**What it does:** Deadman switch / heartbeat monitor. Services or scheduled jobs check in by hitting an endpoint. If a check-in is missed within the configured window, nackmon alerts via Slack.

**Pattern:** A monitored job (e.g., a nightly backup) is configured with an expected check-in interval. If the job runs, it hits `/ack/<guid>`. If nackmon doesn't receive an ack within the window, it fires a Slack alert. This catches silent failures — jobs that stop running without erroring visibly.

**TLS:** None. nackmon runs plain HTTP behind an nginx reverse proxy. Check-in requests from jobs are authenticated by the GUID in the URL path — no additional transport security required on the service itself.

**Implementation:** Review `src/shrmpl_nackmon_srv.rs` in the Rust repo for the full protocol, monitor config format, and alerting logic. Preserve all behavior.

**Monitor config:** Per-monitor `.env` files. Same filename convention as cicd hooks — split on the last hyphen to extract the GUID. The GUID must not contain hyphens.

---

### shrmpl-pulsecheck-srv

**What it does:** HTTP uptime monitor. Polls configured endpoints on a schedule and alerts via Slack if a request fails or returns a non-2xx response.

**Implementation:** Review `src/shrmpl_pulsecheck_srv.rs` in the Rust repo for the full polling logic, endpoint config format, and alerting behavior. Preserve all behavior.

**Endpoint config:** Per-endpoint `.env` files. Same filename convention as cicd hooks — split on the last hyphen to extract the GUID. The GUID must not contain hyphens.

---

## Shared Internal Packages

### internal/config

Shared env file loader used by all five services. No external dependencies. This is an intentional custom implementation — these files are operator-authored with a defined format, not user-submitted input, so the subset of `.env` syntax supported is a documented decision, not a gap.

**Supported format:**

- `KEY=VALUE` — one per line
- Everything after the first `=` is the value (values may contain `=`)
- Leading and trailing whitespace trimmed from both key and value
- Lines starting with `#` are comments and ignored (full-line only — inline comments are not supported)
- Empty values are valid: `KEY=`
- Blank lines are ignored
- Keys with spaces are invalid and should be skipped with a warning

**What is explicitly not supported** (by design):
- Quoted values: `KEY="value with spaces"` — don't use spaces in values
- Multiline values
- Inline comments: `KEY=value # comment`
- `export KEY=VALUE` prefix
- Variable interpolation: `KEY=$OTHER`

**API:**

```go
// Load parses a .env file and returns a map of key-value pairs.
// Returns an error if the file cannot be read.
func Load(path string) (map[string]string, error)

// MustLoad calls Load and exits with a log message if it fails.
func MustLoad(path string) map[string]string

// Get returns a value from the map or a default if the key is absent.
func Get(cfg map[string]string, key, defaultVal string) string

// Require returns a value from the map or exits with a log message if absent.
func Require(cfg map[string]string, key string) string
```

All services load their primary config with `MustLoad`, then use `Require` for mandatory fields and `Get` for optional ones with defaults. This replaces the scattered `config.get(...).expect(...)` pattern in the Rust code.

### internal/logging

Thin wrapper around `log/slog` that:

- Uses the text handler by default
- Suppresses the `time=` field when `LOG_JOURNAL=true`
- Exposes a convenience method for activity/audit events (the ACTV concept)
- Accepts `LOG_LEVEL` to set minimum log level (DEBUG, INFO, WARN, ERROR)

All services initialize logging from this package using their env config.

---

## Open Items

1. **Repo name** — not yet decided. Placeholder used throughout this doc.
2. **Vault cert lifecycle** — how new service certs are provisioned and rotated is not fully specified. Must be documented before vault is considered production-ready.
3. **Expiration format** — confirm the exact syntax accepted by `parse_expiration` in the Rust kv server before implementing in Go (check `src/shrmpl_kv_srv.rs`).
4. **Nackmon and pulsecheck protocol** — these were not analyzed in detail during the review. Read the Rust source before starting the Go implementation.
