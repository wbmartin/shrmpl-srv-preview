# Shrmpl-KV Technical Specification

## Overview
Shrmpl-KV is a lightweight, Redis-compatible key-value store implemented in Rust. It provides two executables: `shrmpl-kv-srv` (server) and `shrmpl-kv-cli` (client). The goal is to offer a minimal subset of Redis commands (GET, SET, INCR, PING, DEL) with backward compatibility, while minimizing dependencies and focusing on simplicity for 3-5 concurrent clients.

Both executables accept two command-line arguments: IP address and port (e.g., `shrmpl-kv-srv 127.0.0.1 4141` for localhost, `shrmpl-kv-srv 0.0.0.0 4141` for network access).

## Supported Commands
- **GET key**: Retrieves the value for the key. Returns the value or an error if not found.
- **SET key value**: Sets the key to the value. Returns "OK" on success or an error on failure (e.g., invalid length).
- **INCR key**: Increments the integer value of the key by 1. If the key doesn't exist or the value isn't an integer, treats it as 0, increments to 1, saves the new value, and returns the incremented number.
- **PING**: No arguments. Returns "PONG".
- **DEL key**: Deletes the key-value pair. Returns "OK" if deleted, or an error if not found.

## Constraints
- Keys and values must be ≤100 characters.
- Only string and integer values are supported.
- No binary data or complex types.
- Designed for low concurrency (3-5 clients).

## Architecture
- **Server**: Async TCP server using Tokio. Listens on specified IP/port. Spawns a task per client connection.
- **Client**: Interactive CLI that maintains a persistent connection to the server. Reads commands from stdin, sends them to the server, and prints responses (ignoring unsolicited UPONG heartbeats).
- **Data Storage**: In-memory `HashMap<String, Value>` where `Value` is an enum (`Int(i64)` or `Str(String)`). Wrapped in `Arc<RwLock<...>>` for concurrency.
- **Concurrency**: Async I/O with Tokio. Read locks for GET, write locks for SET/INCR/DEL. Stale reads are acceptable.
- **Dependencies**: Minimal; std + tokio only. No external crates like dashmap.
- **Connection Management**: Persistent connections (no reconnect per request). TCP_NODELAY enabled. Keepalive set to 60s per socket.

## Protocol
Simple newline-delimited text protocol (not full RESP for simplicity):
- Commands: "COMMAND arg1 arg2\n" (e.g., "GET mykey\n", "SET mykey myvalue\n").
- Responses: "value\n" for data, "OK\n" for success, "ERROR message\n" for errors.
- Pipelining: Client can send multiple commands without waiting; server processes sequentially and streams responses.
- Assumptions: No spaces or newlines in keys/values (enforced by length limits).

## Value Handling
- On SET: If value parses to i64, store as `Int`; else `Str`.
- On GET: Return as string (e.g., "42" for Int, "hello" for Str).
- On INCR: Parse current value as i64 (default 0 if invalid), increment, store as `Int`, return new value as string.

## Heartbeats
- Server sends "UPONG\n" (unsolicited PONG) every 2 minutes per connection to keep NAT/LB alive.
- Client ignores unsolicited PONGs.

## Error Handling
- Invalid key/value lengths: "ERROR invalid length\n"
- Unknown commands: "ERROR unknown command\n"
- INCR on non-integer: Proceeds as 0->1 (no error).
- Network errors: Connection drops.

## Implementation Notes for AI
- Use Tokio runtime: `tokio::net::TcpListener`, `tokio::io::{AsyncReadExt, AsyncWriteExt}`.
- For each connection: Spawn `tokio::spawn(async move { ... })`.
- Read lines with `BufReader::lines()`.
- Parse commands: Split by spaces, match on first word.
- Heartbeats: Use `tokio::time::interval(Duration::from_secs(120))` to send "UPONG\n".
- Data structure: `Arc<RwLock<HashMap<String, Value>>>`.
- Value enum: `enum Value { Int(i64), Str(String) }`.
- Testing: Implement `shrmpl-kv-cli` to send commands and verify responses.
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
