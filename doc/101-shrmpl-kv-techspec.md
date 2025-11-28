# Shrmpl-KV Technical Specification

## Overview
Shrmpl-KV is a lightweight, Redis-compatible key-value store implemented in Rust. It provides two executables: `shrmpl-kv-srv` (server) and `shrmpl-kv-cli` (client). The goal is to offer a minimal subset of Redis commands (GET, SET, INCR, PING, DEL) with backward compatibility, while minimizing dependencies and focusing on simplicity for 3-5 concurrent clients.

Both executables accept two command-line arguments: IP address and port (e.g., `shrmpl-kv-srv 127.0.0.1 4141` for localhost, `shrmpl-kv-srv 0.0.0.0 4141` for network access).

## Supported Commands
- **GET key**: Retrieves the value for the key. Returns the value or an error if not found. Automatically removes expired keys.
- **SET key value [expiration]**: Sets the key to the value with optional expiration. Returns "OK" on success or an error on failure. Expiration formats: "30s", "5min", "1h".
- **INCR key [expiration]**: Increments the integer value of the key by 1 with optional expiration. If the key doesn't exist or the value isn't an integer, treats it as 0, increments to 1, saves the new value, and returns the incremented number.
- **LIST**: Lists all keys in the memory store with their values and expiration times. Returns one line per key in insertion order.
- **PING**: No arguments. Returns "PONG".
- **DEL key**: Deletes the key-value pair. Returns "OK" if deleted, or an error if not found.

## Constraints
- Keys and values must be ≤100 characters.
- Only string and integer values are supported.
- No binary data or complex types.
- Designed for low concurrency (3-5 clients).

## Architecture
- **Server**: Async TCP server using Tokio. Listens on specified IP/port. Spawns a task per client connection. Includes background cleanup task for expired keys.
- **Client**: Interactive CLI that maintains a persistent connection to the server. Reads commands from stdin, sends them to the server, and prints responses (ignoring unsolicited UPONG heartbeats).
- **Data Storage**: In-memory `HashMap<String, StoredValue>` where `StoredValue` contains `Value` enum (`Int(i64)` or `Str(String)`) and optional `expires_at` timestamp. Wrapped in `Arc<RwLock<...>>` for concurrency.
- **Concurrency**: Async I/O with Tokio. Write locks for all operations (GET needs write lock for expiration cleanup). Background cleanup runs every 60 seconds.
- **Dependencies**: Minimal; std + tokio only. No external crates like dashmap.
- **Connection Management**: Persistent connections (no reconnect per request). TCP_NODELAY enabled. Keepalive set to 60s per socket.
- **Expiration**: Keys can have TTLs set via SET/INCR commands. Expired keys are removed on access and by background cleanup task.

## Protocol
Simple newline-delimited text protocol (not full RESP for simplicity):
- Commands: "COMMAND arg1 arg2 [arg3]\n" (e.g., "GET mykey\n", "SET mykey myvalue 5min\n", "INCR counter 1h\n", "LIST\n").
- Responses: "value\n" for data, "OK\n" for success, "ERROR message\n" for errors.
- LIST Response Format: "key=value,expiration_timestamp\n" per line, in insertion order. Expiration timestamp is ISO8601 UTC or "no-expiration" for keys without TTL.
- Pipelining: Client can send multiple commands without waiting; server processes sequentially and streams responses.
- Assumptions: No spaces or newlines in keys/values (enforced by length limits).
- Expiration formats: "30s" (seconds), "5min" (minutes), "1h" (hours).

## Value Handling
- On SET: If value parses to i64, store as `Int`; else `Str`. Optional expiration sets `expires_at` timestamp.
- On GET: Check expiration first. If expired, remove key and return "ERROR key not found". Otherwise return value as string (e.g., "42" for Int, "hello" for Str).
- On INCR: Check expiration first. If expired, treat as new key. Parse current value as i64 (default 0 if invalid), increment, store as `Int` with optional expiration, return new value as string.
- Expiration parsing: Supports "30s" (30 seconds), "5min" (5 minutes), "1h" (1 hour). Invalid expiration formats cause "ERROR invalid expiration\n".

## Heartbeats
- Server sends "UPONG\n" (unsolicited PONG) every 2 minutes per connection to keep NAT/LB alive.
- Client ignores unsolicited PONGs.

## Error Handling
- Invalid key/value lengths: "ERROR invalid length\n"
- Invalid expiration format: "ERROR invalid expiration\n"
- Unknown commands: "ERROR unknown command\n"
- INCR on non-integer: Proceeds as 0->1 (no error).
- Expired keys: Treated as not found on access.
- Network errors: Connection drops.

## Implementation Notes for AI
- Use Tokio runtime: `tokio::net::TcpListener`, `tokio::io::{AsyncReadExt, AsyncWriteExt}`.
- For each connection: Spawn `tokio::spawn(async move { ... })`.
- Read lines with `BufReader::lines()`.
- Parse commands: Split by spaces, match on first word.
- Heartbeats: Use `tokio::time::interval(Duration::from_secs(120))` to send "UPONG\n".
- Data structure: `Arc<RwLock<HashMap<String, StoredValue>>>` where `StoredValue { value: Value, expires_at: Option<Instant> }`.
- Value enum: `enum Value { Int(i64), Str(String) }`.
- Expiration: Use `std::time::Instant` for timestamps. Background cleanup task runs every 60 seconds with `tokio::time::interval`.
- GET operations: Use write lock to allow immediate cleanup of expired keys.
- Expiration parsing: Parse suffixes "s", "min", "h" and convert to `Duration`, then calculate `Instant::now() + duration`.
- Testing: Implement `shrmpl-kv-cli` to send commands and verify responses, including expiration behavior.
- Build with `cargo build --release` for `shrmpl-kv-srv` and `shrmpl-kv-cli` binaries.

## Build and Distribution Strategy
Optimized for a small team (3-5 devs) developing on Mac (Apple Silicon) and deploying to Debian.

### Development Setup (Mac Apple Silicon)
- **Local Builds**: Use Cargo natively (`cargo build` for debug, `cargo build --release` for optimized). Rust's aarch64-apple-darwin target works out-of-the-box.
- **Dependencies**: Ensure Rust toolchain via rustup. No extra setup.
- **Testing**: Run `cargo test` locally. Use `shrmpl-kv-cli` against local `shrmpl-kv-srv`.

### Production Builds (Debian)
- **Target**: x86_64-unknown-linux-gnu (common); adjust for ARM if needed.
- **Cross-Compilation**: From Mac, add target `rustup target add x86_64-unknown-linux-gnu`, install linker (e.g., `brew install llvm`), build with `cargo build --release --target x86_64-unknown-linux-gnu`.
- **Alternative**: Use Docker (`rust:slim` image) for reproducible builds.
- **CI/CD**: GitHub Actions with matrix builds for both targets; push to releases.

### Distribution
- **Binaries**: Release via GitHub Releases with `shrmpl-kv-srv` and `shrmpl-kv-cli` executables.
- **Versioning**: Semantic tags (e.g., v1.0.0).
- **Installation**: Download, `chmod +x`, move to `/usr/local/bin`.
- **Team Sharing**: Git for source; binaries via shared drive or `scp`.
- **Updates**: Manual notifications; no auto-updates.
- **Security**: Optional GPG signing.
