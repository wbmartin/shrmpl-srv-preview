const VERSION: &str = env!("CARGO_PKG_VERSION");

use crate::shrmpl_log_client::Logger;
use shrmpl::{config, shrmpl_log_client};
use socket2::{Socket, TcpKeepalive};
use std::collections::HashMap;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, Duration as TokioDuration};

#[derive(Clone, Debug)]
enum Value {
    Int(i64),
    Str(String),
}

#[derive(Clone, Debug)]
struct StoredValue {
    value: Value,
    expires_at: Option<SystemTime>,
}

type KvStore = Arc<RwLock<HashMap<String, StoredValue>>>;

fn parse_expiration(exp_str: &str) -> Option<Duration> {
    if exp_str.ends_with("s") {
        let num_str = exp_str.trim_end_matches('s');
        num_str.parse::<u64>().ok().map(Duration::from_secs)
    } else if exp_str.ends_with("min") {
        let num_str = exp_str.trim_end_matches("min");
        num_str.parse::<u64>().ok().map(|secs| Duration::from_secs(secs * 60))
    } else if exp_str.ends_with("h") {
        let num_str = exp_str.trim_end_matches('h');
        num_str.parse::<u64>().ok().map(|hours| Duration::from_secs(hours * 3600))
    } else {
        None
    }
}

// Server application uses fail-fast approach with expect()/unwrap() for startup errors
// since server processes should fail immediately on configuration or socket setup issues
// and be restarted by process managers rather than attempting graceful recovery
#[tokio::main]
async fn main() {
    println!("shrmpl-kv-srv version {}", VERSION);
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }
    let config_path = &args[1];
    // Config loading uses expect() because missing critical config values should cause
    // immediate server failure - these are not recoverable runtime errors
    let config = config::load_config(config_path);
    let send_log = config.get("SEND_LOG").map(|s| s == "true").unwrap_or(false);
    // Critical configuration values use expect() - server cannot function without these
    let bind_addr = config
        .get("BIND_ADDR")
        .expect("BIND_ADDR not found in config")
        .clone();
    let slog_dest = config.get("SLOG_DEST").cloned().unwrap_or_default();
    let server_name = config
        .get("SERVER_NAME")
        .cloned()
        .unwrap_or_else(|| "skv-srv".to_string());

    // Load new logging configuration
    let log_level =
        shrmpl_log_client::LogLevel::from_str(config.get("LOG_LEVEL").map_or("INFO", |v| v.as_str()));
    let log_console = config
        .get("LOG_CONSOLE")
        .map(|s| s == "true")
        .unwrap_or(true);
    let send_actv = config
        .get("SEND_ACTV")
        .map(|s| s == "true")
        .unwrap_or(false);

    let logger =
        shrmpl_log_client::Logger::new(slog_dest, server_name, log_level, log_console, send_actv, send_log);
    let addr_parts: Vec<&str> = bind_addr.split(':').collect();
    if addr_parts.len() != 2 {
        logger
            .error("KVINVALIDBND", "Invalid BIND_ADDR format")
            .await;
        std::process::exit(1);
    }
    let ip = addr_parts[0];
    let port = addr_parts[1];
    let addr = format!("{}:{}", ip, port);

    // Socket setup uses expect() - these are system-level failures that should crash
    // the server process immediately rather than attempting to continue in a broken state
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)
        .expect("Failed to create socket");
    socket.set_keepalive(true).expect("Failed to set keepalive");
    socket
        .set_tcp_keepalive(&TcpKeepalive::new().with_time(Duration::from_secs(60)))
        .expect("Failed to set tcp keepalive");
    socket
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");
    let addr_parsed: std::net::SocketAddr = addr.parse().expect("Invalid address");
    socket.bind(&addr_parsed.into()).expect("Failed to bind");
    socket.listen(128).expect("Failed to listen");
    let std_listener: StdTcpListener = socket.into();
    let listener = TcpListener::from_std(std_listener).expect("Failed to convert listener");
    logger
        .info("KVSERVERLIST", &format!("shrmpl-kv-srv version {} listening on {}", VERSION, addr))
        .await;

    let store: KvStore = Arc::new(RwLock::new(HashMap::new()));
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Spawn cleanup task for expired keys
    let store_for_cleanup = store.clone();
    let cleanup_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut cleanup_interval = interval(TokioDuration::from_secs(60));
        let mut shutdown_rx = cleanup_shutdown_rx;
        loop {
            tokio::select! {
                _ = cleanup_interval.tick() => {
                    let mut store_write = store_for_cleanup.write().await;
                    let now = SystemTime::now();
                    store_write.retain(|_, stored_value| {
                        match stored_value.expires_at {
                            Some(exp_time) => exp_time > now,
                            None => true,
                        }
                    });
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });

    // Spawn shutdown handler
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        let _ = shutdown_tx_clone.send(());
    });

    let mut shutdown_rx = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (socket, _) = accept_result.expect("Failed to accept");
                let store = store.clone();
                let conn_shutdown_rx = shutdown_tx.subscribe();
                let logger_clone = logger.clone();
                tokio::spawn(async move {
                    handle_connection(socket, store, conn_shutdown_rx, logger_clone).await;
                });
            }
            _ = shutdown_rx.recv() => {
                logger.info("KVSERVERDOWN", "Shutting down server...").await;
                break;
            }
        }
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    store: KvStore,
    mut shutdown_rx: broadcast::Receiver<()>,
    logger: Logger,
) {
    // Set TCP_NODELAY
    socket.set_nodelay(true).unwrap_or_default();

    let (reader, mut writer) = socket.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Heartbeat interval: send UPONG every 2 minutes
    let mut heartbeat = interval(Duration::from_secs(120));

    loop {
        line.clear();
        tokio::select! {
            _ = heartbeat.tick() => {
                if writer.write_all(b"UPONG\n").await.is_err() {
                    return; // Connection closed
                }
            }
            result = reader.read_line(&mut line) => {
                match result {
                    Ok(0) => return, // EOF
                    Ok(_) => {
                        let trimmed = line.trim_end();
                        if !trimmed.is_empty() {
                              logger.debug("KVCMDRECV", &format!("Received command: {}", trimmed)).await;
                            let response = process_command(trimmed, &store, &logger).await;
                            if writer.write_all(response.as_bytes()).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(_) => return,
                }
            }
            _ = shutdown_rx.recv() => {
                let _ = writer.write_all(b"TERM\n").await;
                return;
            }
        }
    }
}

async fn process_single_command(
    parts: Vec<&str>,
    store: &KvStore,
) -> String {
    if parts.is_empty() {
        return "ERROR unknown command\n".to_string();
    }

    let cmd = parts[0];

    match cmd {
        "PING" => "PONG\n".to_string(),
        "GET" => {
            if parts.len() != 2 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            if key.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }
            let mut store_write = store.write().await;
            match store_write.get(key) {
                Some(stored) => {
                    if let Some(exp_time) = stored.expires_at {
                        if exp_time <= SystemTime::now() {
                            store_write.remove(key);
                            "ERROR key not found\n".to_string()
                        } else {
                            match &stored.value {
                                Value::Int(i) => format!("{}\n", i),
                                Value::Str(s) => format!("{}\n", s),
                            }
                        }
                    } else {
                        match &stored.value {
                            Value::Int(i) => format!("{}\n", i),
                            Value::Str(s) => format!("{}\n", s),
                        }
                    }
                }
                 None => "*KEY NOT FOUND*\n".to_string(),
            }
        }
        "SET" => {
            if parts.len() < 3 || parts.len() > 4 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            let value_str = parts[2];
            if key.len() > 100 || value_str.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }

            let expires_at = if parts.len() == 4 {
                let exp_str = parts[3];
                if let Some(duration) = parse_expiration(exp_str) {
                    Some(SystemTime::now() + duration)
                } else {
                    return "ERROR invalid expiration\n".to_string();
                }
            } else {
                None
            };

            let value = if let Ok(i) = value_str.parse::<i64>() {
                Value::Int(i)
            } else {
                Value::Str(value_str.to_string())
            };

            let stored_value = StoredValue { value, expires_at };
            let mut store_write = store.write().await;
            store_write.insert(key.to_string(), stored_value);
            "OK\n".to_string()
        }
        "INCR" => {
            if parts.len() < 2 || parts.len() > 3 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            if key.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }

            let mut store_write = store.write().await;
            let current = store_write.get(key);
            let new_val = match current {
                Some(stored) => {
                    if let Some(exp_time) = stored.expires_at {
                        if exp_time <= SystemTime::now() {
                            1 // Expired, treat as new
                        } else {
                            match &stored.value {
                                Value::Int(i) => i + 1,
                                Value::Str(_) => 1, // Treat as 0, increment to 1
                            }
                        }
                    } else {
                        match &stored.value {
                            Value::Int(i) => i + 1,
                            Value::Str(_) => 1, // Treat as 0, increment to 1
                        }
                    }
                }
                None => 1, // New key
            };

            // Only set expiration if the key is new (None case)
            let expires_at = if parts.len() == 3 && current.is_none() {
                let exp_str = parts[2];
                if let Some(duration) = parse_expiration(exp_str) {
                    Some(SystemTime::now() + duration)
                } else {
                    return "ERROR invalid expiration\n".to_string();
                }
            } else {
                // Keep existing expiration or none
                current.and_then(|stored| stored.expires_at)
            };

            let stored_value = StoredValue {
                value: Value::Int(new_val),
                expires_at,
            };
            store_write.insert(key.to_string(), stored_value);
            format!("{}\n", new_val)
        }
        "DEL" => {
            if parts.len() != 2 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            if key.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }
            let mut store_write = store.write().await;
            match store_write.get(key) {
                Some(stored) => {
                    if let Some(exp_time) = stored.expires_at {
                        if exp_time <= SystemTime::now() {
                            store_write.remove(key);
                            "ERROR key not found\n".to_string()
                        } else {
                            store_write.remove(key);
                            "OK\n".to_string()
                        }
                    } else {
                        store_write.remove(key);
                        "OK\n".to_string()
                    }
                }
                 None => "*KEY NOT FOUND*\n".to_string(),
            }
        }
        "LIST" => {
            if parts.len() != 1 {
                return "ERROR invalid arguments\n".to_string();
            }
            let store_read = store.read().await;
            let mut result = String::new();
            for (key, stored_value) in store_read.iter() {
                let value_str = match &stored_value.value {
                    Value::Int(i) => i.to_string(),
                    Value::Str(s) => s.clone(),
                };
                let expiration_str = match stored_value.expires_at {
                    Some(exp_time) => {
                        let timestamp = exp_time.duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        timestamp.to_string()
                    }
                    None => "no-expiration".to_string(),
                };
                result.push_str(&format!("{}={},{}\n", key, value_str, expiration_str));
            }
            if result.is_empty() {
                "\n".to_string()
            } else {
                result
            }
        }
        _ => "ERROR unknown command\n".to_string(),
    }
}

async fn process_command(
    line: &str,
    store: &KvStore,
    logger: &shrmpl_log_client::Logger,
) -> String {
    let result = if line.starts_with("BATCH ") {
        let batch_commands = &line[6..]; // Skip "BATCH "
        let commands: Vec<&str> = batch_commands.split(';').collect();
        if commands.len() > 3 {
            "ERROR too many commands\n".to_string()
        } else {
            let mut results = Vec::new();
            for cmd in commands {
                let trimmed = cmd.trim();
                if !trimmed.is_empty() {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    let result = process_single_command(parts, store).await;
                    let clean_result = result.trim_end();
                    results.push(clean_result.to_string());
                }
            }
            results.join(";") + "\n"
        }
    } else {
        let parts: Vec<&str> = line.split_whitespace().collect();
        process_single_command(parts, store).await
    };

    logger.debug("KVCMDPROC", &format!("Processing command: {} = {}", line.trim(), result.trim())).await;
    result
}
