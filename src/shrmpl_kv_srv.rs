use crate::shrmpl_log_client::Logger;
use shrmpl::{config, shrmpl_log_client};
use socket2::{Socket, TcpKeepalive};
use std::collections::HashMap;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, Duration};

#[derive(Clone, Debug)]
enum Value {
    Int(i64),
    Str(String),
}

type KvStore = Arc<RwLock<HashMap<String, Value>>>;

// Server application uses fail-fast approach with expect()/unwrap() for startup errors
// since server processes should fail immediately on configuration or socket setup issues
// and be restarted by process managers rather than attempting graceful recovery
#[tokio::main]
async fn main() {
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
            .error("AAEA", "Invalid BIND_ADDR format")
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
        .info("AAAA", &format!("Server listening on {}", addr))
        .await;

    let store: KvStore = Arc::new(RwLock::new(HashMap::new()));
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

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
                logger.info("AAAA", "Shutting down server...").await;
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
                             logger.debug("AABA", &format!("Received command: {}", trimmed)).await;
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

async fn process_command(
    line: &str,
    store: &KvStore,
    logger: &shrmpl_log_client::Logger,
) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return "ERROR unknown command\n".to_string();
    }

    let cmd = parts[0].to_string();

    logger
        .debug("AABC", &format!("Processing command: {}", line))
        .await;

    match cmd.as_str() {
        "PING" => "PONG\n".to_string(),
        "GET" => {
            if parts.len() != 2 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            if key.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }
            let store_read = store.read().await;
            match store_read.get(key) {
                Some(Value::Int(i)) => format!("{}\n", i),
                Some(Value::Str(s)) => format!("{}\n", s),
                None => "ERROR key not found\n".to_string(),
            }
        }
        "SET" => {
            if parts.len() != 3 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            let value_str = parts[2];
            if key.len() > 100 || value_str.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }
            let value = if let Ok(i) = value_str.parse::<i64>() {
                Value::Int(i)
            } else {
                Value::Str(value_str.to_string())
            };
            let mut store_write = store.write().await;
            store_write.insert(key.to_string(), value);
            "OK\n".to_string()
        }
        "INCR" => {
            if parts.len() != 2 {
                return "ERROR invalid arguments\n".to_string();
            }
            let key = parts[1];
            if key.len() > 100 {
                return "ERROR invalid length\n".to_string();
            }
            let mut store_write = store.write().await;
            let current = store_write.get(key).cloned().unwrap_or(Value::Int(0));
            let new_val = match current {
                Value::Int(i) => i + 1,
                Value::Str(_) => 1, // Treat as 0, increment to 1
            };
            store_write.insert(key.to_string(), Value::Int(new_val));
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
            if store_write.remove(key).is_some() {
                "OK\n".to_string()
            } else {
                "ERROR key not found\n".to_string()
            }
        }
        _ => "ERROR unknown command\n".to_string(),
    }
}
