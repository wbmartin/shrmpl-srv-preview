use std::fs;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use chrono::Utc;
use crossbeam_channel::{bounded, Receiver, Sender};
use shrmpl::config;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};

#[derive(Clone)]
struct Record {
    lvl: [u8; 4],
    host: [u8; 32],
    code: [u8; 4],
    len: u16,
    msg: Vec<u8>,
    recv_ts: [u8; 24],
}

struct Config {
    data_dir: String,
    bind_addr: String,
    dev_mode: bool,
    queue_capacity: usize,
}

struct Counters {
    received: AtomicU64,
    dropped: AtomicU64,
    oversize: AtomicU64,
    activity_written: AtomicU64,
    error_written: AtomicU64,
    misc_written: AtomicU64,
    protocol_errors: AtomicU64,
}



fn get_queue(lvl: &[u8; 4]) -> usize {
    if lvl == b"ACTV" {
        0
    } else if lvl == b"ERRO" {
        1
    } else {
        2
    }
}

enum ParseError {
    Invalid,
    Oversize,
}

// Protocol parsing uses custom error types for precise error categorization
// (Invalid vs Oversize) to enable different handling strategies in calling code
fn parse_line(line: &[u8]) -> Result<Record, ParseError> {
    if line.len() < 50 || line.last() != Some(&b'\n') {
        return Err(ParseError::Invalid);
    }
    let lvl: [u8; 4] = line[0..4].try_into().map_err(|_| ParseError::Invalid)?;
    let host: [u8; 32] = line[5..37].try_into().map_err(|_| ParseError::Invalid)?;
    let code: [u8; 4] = line[38..42].try_into().map_err(|_| ParseError::Invalid)?;
    let len_str = std::str::from_utf8(&line[43..47]).map_err(|_| ParseError::Invalid)?;
    let len: u16 = len_str.parse().map_err(|_| ParseError::Invalid)?;
    if len > 4096 {
        return Err(ParseError::Oversize);
    }
    if line.len() != 49 + len as usize + 1 {
        return Err(ParseError::Invalid);
    }
    let msg = line[49..49 + len as usize].to_vec();
    let recv_ts = Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
        .into_bytes();
    let mut recv_ts_arr = [0u8; 24];
    recv_ts_arr.copy_from_slice(&recv_ts[..24]);
    Ok(Record {
        lvl,
        host,
        code,
        len,
        msg,
        recv_ts: recv_ts_arr,
    })
}

async fn handle_connection(
    socket: TcpStream,
    tx_activity: Sender<Record>,
    tx_error: Sender<Record>,
    tx_misc: Sender<Record>,
    counters: Arc<Counters>,
    _dev_mode: bool,
    mut keepalive_rx: tokio::sync::broadcast::Receiver<String>,
) {
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    loop {
        line.clear();
        tokio::select! {
            result = reader.read_line(&mut line) => {
                match result {
                    Ok(0) => return,
                    Ok(_) => {
                        let line_bytes = line.as_bytes();
                        match parse_line(line_bytes) {
                            Ok(record) => {
                                println!("Received message: lvl={}, host={}, code={}, msg={}", String::from_utf8_lossy(&record.lvl), String::from_utf8_lossy(&record.host), String::from_utf8_lossy(&record.code),String::from_utf8_lossy(&record.msg));
                                counters.received.fetch_add(1, Ordering::Relaxed);
                                let queue = get_queue(&record.lvl);
                                let sent = if queue == 0 {
                                    tx_activity.try_send(record)
                                } else if queue == 1 {
                                    tx_error.try_send(record)
                                } else {
                                    tx_misc.try_send(record)
                                };
                                if sent.is_err() {
                                    counters.dropped.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(ParseError::Invalid) => {
                                println!("Protocol error: invalid log message format");
                                counters.protocol_errors.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(ParseError::Oversize) => {
                                println!("Protocol error: log message too large (>4096 bytes)");
                                counters.oversize.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(_) => return,
                }
            }
            msg = keepalive_rx.recv() => {
                if let Ok(msg) = msg {
                    let _ = reader.get_mut().write_all(msg.as_bytes()).await;
                }
            }
        }
    }
}

fn start_writers(
    rx_activity: Receiver<Record>,
    rx_error: Receiver<Record>,
    rx_misc: Receiver<Record>,
    data_dir: String,
    counters: Arc<Counters>,
    _dev_mode: bool,
) {
    let data_dir1 = data_dir.clone();
    let counters1 = counters.clone();
    std::thread::spawn(move || {
        writer_loop(
            rx_activity,
            "activity",
            &data_dir1,
            &counters1.activity_written,
        )
    });
    let data_dir2 = data_dir.clone();
    let counters2 = counters.clone();
    std::thread::spawn(move || {
        writer_loop(rx_error, "error", &data_dir2, &counters2.error_written)
    });
    let counters3 = counters.clone();
    std::thread::spawn(move || writer_loop(rx_misc, "misc", &data_dir, &counters3.misc_written));
}

fn writer_loop(rx: Receiver<Record>, file_prefix: &str, data_dir: &str, counter: &AtomicU64) {
    let mut current_date = String::new();
    let mut writer: Option<BufWriter<fs::File>> = None;
    let mut last_flush = std::time::Instant::now();
    loop {
        match rx.recv() {
            Ok(record) => {
                let date = std::str::from_utf8(&record.recv_ts[..10])
                    .unwrap()
                    .replace("-", "");
                if date != current_date {
                    writer = Some(open_file(data_dir, file_prefix, &date));
                    current_date = date.clone();
                }
                if let Some(ref mut w) = writer {
                    // High-frequency log writing uses unwrap() for performance:
                    // - These operations should never fail in normal operation
                    // - If they do fail, it indicates serious disk/system issues
                    // - Panicking is appropriate since the log writer cannot recover
                    w.write_all(&record.recv_ts).unwrap();
                    w.write_all(b" ").unwrap();
                    w.write_all(&record.lvl).unwrap();
                    w.write_all(b" ").unwrap();
                    w.write_all(&record.host).unwrap();
                    w.write_all(b" ").unwrap();
                    w.write_all(&record.code).unwrap();
                    w.write_all(b" ").unwrap();
                    write!(w, "{:04}", record.len).unwrap();
                    w.write_all(b": ").unwrap();
                    w.write_all(&record.msg).unwrap();
                    w.write_all(b"\n").unwrap();
                    counter.fetch_add(1, Ordering::Relaxed);
                    if last_flush.elapsed() > Duration::from_secs(2) {
                        // Flush operations use unwrap() - failure to flush indicates
                        // serious disk issues that should cause the writer thread to panic
                        w.flush().unwrap();
                        w.get_ref().sync_data().unwrap();
                        last_flush = std::time::Instant::now();
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn open_file(data_dir: &str, prefix: &str, date: &str) -> BufWriter<fs::File> {
    let path = format!("{}/{}-{}.log", data_dir, prefix, date);
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    BufWriter::new(file)
}

async fn signal_handler(counters: Arc<Counters>) {
    let mut sigusr1 =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).unwrap();
    loop {
        sigusr1.recv().await;
        println!(
            "Counters: received={}, dropped={}, oversize={}, activity_written={}, error_written={}, misc_written={}, protocol_errors={}",
            counters.received.load(Ordering::Relaxed),
            counters.dropped.load(Ordering::Relaxed),
            counters.oversize.load(Ordering::Relaxed),
            counters.activity_written.load(Ordering::Relaxed),
            counters.error_written.load(Ordering::Relaxed),
            counters.misc_written.load(Ordering::Relaxed),
            counters.protocol_errors.load(Ordering::Relaxed),
        );
    }
}

// Log server uses mixed error handling: proper propagation for setup operations
// but unwrap() in high-frequency worker threads where performance is critical
// and errors indicate serious system issues that should cause immediate failure
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "etc/slog.env".to_string());

    let map = config::load_config(&config_path);
    let config = Config {
        data_dir: map.get("DATA_DIR").ok_or("DATA_DIR missing")?.clone(),
        bind_addr: map.get("BIND_ADDR").ok_or("BIND_ADDR missing")?.clone(),
        dev_mode: map
            .get("DEV_MODE")
            .map(|s| s.parse().unwrap_or(false))
            .unwrap_or(false),
        queue_capacity: map
            .get("QUEUE_CAPACITY")
            .map(|s| s.parse().unwrap_or(10000))
            .unwrap_or(10000),
    };
    std::fs::create_dir_all(&config.data_dir)?;

    let counters = Arc::new(Counters {
        received: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        oversize: AtomicU64::new(0),
        activity_written: AtomicU64::new(0),
        error_written: AtomicU64::new(0),
        misc_written: AtomicU64::new(0),
        protocol_errors: AtomicU64::new(0),
    });
    let (tx_activity, rx_activity) = bounded(config.queue_capacity / 3);
    let (tx_error, rx_error) = bounded(config.queue_capacity / 3);
    let (tx_misc, rx_misc) = bounded(config.queue_capacity / 3);
    let (keepalive_tx, _) = broadcast::channel::<String>(10);

    start_writers(
        rx_activity,
        rx_error,
        rx_misc,
        config.data_dir.clone(),
        counters.clone(),
        config.dev_mode,
    );

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    println!("Listening on {}", config.bind_addr);

    let start_time = Utc::now();

    tokio::spawn(signal_handler(counters.clone()));

    let start_time_clone = start_time;
    let counters_clone = counters.clone();
    let tx_misc_clone = tx_misc.clone();
    let keepalive_tx_clone = keepalive_tx.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let unix_millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let msg = format!("UPONG {}\n", unix_millis);
            let _ = keepalive_tx_clone.send(msg);

            let uptime = Utc::now()
                .signed_duration_since(start_time_clone)
                .num_seconds() as f64
                / 3600.0;
            let stats_msg = format!("recv={} dropped={} oversize={} activity_written={} error_written={} misc_written={} protocol_errors={} uptime={:.2}h",
                counters_clone.received.load(Ordering::Relaxed),
                counters_clone.dropped.load(Ordering::Relaxed),
                counters_clone.oversize.load(Ordering::Relaxed),
                counters_clone.activity_written.load(Ordering::Relaxed),
                counters_clone.error_written.load(Ordering::Relaxed),
                counters_clone.misc_written.load(Ordering::Relaxed),
                counters_clone.protocol_errors.load(Ordering::Relaxed),
                uptime
            );
            let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
            let host = format!("{:32}", "server.local");
            let _code = "STAT";
            let _len = format!("{:04}", stats_msg.len());

            println!("Stats: {}", stats_msg);
            let record = Record {
                lvl: *b"INFO",
                host: host.as_bytes().try_into().unwrap(),
                code: *b"STAT",
                len: stats_msg.len() as u16,
                msg: stats_msg.into_bytes(),
                recv_ts: timestamp.as_bytes().try_into().unwrap_or([0; 24]),
            };
            let _ = tx_misc_clone.try_send(record);
        }
    });

    loop {
        let (socket, _) = listener.accept().await?;
        let tx_activity = tx_activity.clone();
        let tx_error = tx_error.clone();
        let tx_misc = tx_misc.clone();
        let counters = counters.clone();
        let dev_mode = config.dev_mode;
        let local_tx = keepalive_tx.clone();
        tokio::spawn(async move {
            let keepalive_rx = local_tx.subscribe();
            handle_connection(
                socket,
                tx_activity,
                tx_error,
                tx_misc,
                counters,
                dev_mode,
                keepalive_rx,
            )
            .await;
        });
    }
}
