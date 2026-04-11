const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use croner::Cron;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use shrmpl::config::load_config;
use shrmpl::shrmpl_log_client::Logger;

// --- Data types ---

#[allow(dead_code)]
struct ServerConfig {
    listen_addr: String,
    listen_port: u16,
    monitors_dir: String,
    status_path: String,
    server_name: String,
}

#[derive(Clone)]
#[allow(dead_code)]
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

#[derive(Clone)]
struct MonitorState {
    alert_count: u32,
    last_checkin: Option<DateTime<Utc>>,
    last_alert: Option<DateTime<Utc>>,
}

#[allow(dead_code)]
struct AppState {
    config: ServerConfig,
    monitors: HashMap<String, MonitorConfig>,
    monitor_states: Mutex<HashMap<String, MonitorState>>,
    start_time: DateTime<Utc>,
    logger: Logger,
}

// --- Request handling ---

async fn handle_request(
    req: Request<Body>,
    state: Arc<AppState>,
) -> Result<Response<Body>, hyper::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    match (&method, path.as_str()) {
        (&Method::GET, "/health") => handle_health(&state).await,
        (&Method::GET, "/checkin") => handle_checkin(&query, &state).await,
        (&Method::GET, "/ack") => handle_ack(&query, &state).await,
        (&Method::GET, p) if p == state.config.status_path => handle_status(&state).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap()),
    }
}

async fn handle_health(state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let uptime = (Utc::now() - state.start_time).num_seconds().max(0) as u64;
    let body = format!(
        r#"{{"status":"ok","monitors_loaded":{},"uptime_seconds":{}}}"#,
        state.monitors.len(),
        uptime
    );
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

/// Parse and validate the `code` query parameter. Returns the code string
/// or an HTTP error response.
fn parse_code_param(query: &str) -> Result<String, Response<Body>> {
    let query = if query.len() > 2048 { &query[..2048] } else { query };
    let code = query
        .split('&')
        .take(32)
        .find_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            match (parts.next(), parts.next()) {
                (Some("code"), Some(v)) => Some(v.to_string()),
                _ => None,
            }
        });

    match code {
        Some(c) if c.len() >= 4 && c.len() <= 25 => Ok(c),
        Some(_) => Err(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(r#"{"error":"code must be 4-25 characters"}"#))
            .unwrap()),
        None => Err(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(r#"{"error":"missing code parameter"}"#))
            .unwrap()),
    }
}

async fn handle_checkin(query: &str, state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let code = match parse_code_param(query) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    if !state.monitors.contains_key(&code) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(r#"{"error":"unknown code"}"#))
            .unwrap());
    }

    let monitor = &state.monitors[&code];
    let now = Utc::now();

    let mut states = state.monitor_states.lock().await;
    let ms = states.entry(code.clone()).or_insert(MonitorState {
        alert_count: 0,
        last_checkin: None,
        last_alert: None,
    });

    let had_active_alarm = ms.alert_count > 0;
    ms.last_checkin = Some(now);
    ms.alert_count = 0;

    state
        .logger
        .info(
            "NACKCHKIN",
            &format!("Check-in received: code={} name={}", code, monitor.name),
        )
        .await;

    // Notify Slack that the alarm has cleared
    if had_active_alarm {
        if let Some(ref url) = monitor.slack_webhook {
            let suffix = monitor
                .slack_message
                .as_deref()
                .map(|m| format!(" {}", m))
                .unwrap_or_default();
            let msg = format!(
                ":white_check_mark: [{}] `{}` (code={}) checked in — alarm cleared.{}",
                state.config.server_name, monitor.name, code, suffix
            );
            send_slack(url, &msg, &state.logger).await;
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

async fn handle_ack(query: &str, state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let code = match parse_code_param(query) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    if !state.monitors.contains_key(&code) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(r#"{"error":"unknown code"}"#))
            .unwrap());
    }

    let monitor = &state.monitors[&code];
    let now = Utc::now();

    let mut states = state.monitor_states.lock().await;
    let ms = states.entry(code.clone()).or_insert(MonitorState {
        alert_count: 0,
        last_checkin: None,
        last_alert: None,
    });

    let had_active_alarm = ms.alert_count > 0;
    ms.last_checkin = Some(now);
    ms.alert_count = 0;

    state
        .logger
        .info(
            "NACKACK",
            &format!("Alarm acknowledged: code={} name={}", code, monitor.name),
        )
        .await;

    if had_active_alarm {
        if let Some(ref url) = monitor.slack_webhook {
            let suffix = monitor
                .slack_message
                .as_deref()
                .map(|m| format!(" {}", m))
                .unwrap_or_default();
            let msg = format!(
                ":no_bell: [{}] `{}` (code={}) alarm silenced by operator.{}",
                state.config.server_name, monitor.name, code, suffix
            );
            send_slack(url, &msg, &state.logger).await;
        }
    }

    let body = if had_active_alarm {
        r#"{"status":"acknowledged"}"#
    } else {
        r#"{"status":"no active alarm"}"#
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

async fn handle_status(state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let states = state.monitor_states.lock().await;
    let mut entries = Vec::new();

    for (code, monitor) in &state.monitors {
        let ms = states.get(code);
        let misses = ms.map(|s| s.alert_count).unwrap_or(0);
        let last_checkin = ms
            .and_then(|s| s.last_checkin)
            .map(|t| format!("\"{}\"", t.format("%Y-%m-%dT%H:%M:%SZ")))
            .unwrap_or_else(|| "null".to_string());

        entries.push(format!(
            r#"{{"code":"{}","name":"{}","description":"{}","alert_count":{},"last_checkin":{}}}"#,
            code, monitor.name, monitor.description, misses, last_checkin
        ));
    }

    let body = format!("[{}]", entries.join(","));
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

// --- Slack notifications ---

async fn send_slack(webhook_url: &str, message: &str, logger: &Logger) {
    let payload = format!(
        r#"{{"text":"{}"}}"#,
        message
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    );

    let uri: hyper::Uri = match webhook_url.parse() {
        Ok(u) => u,
        Err(e) => {
            logger
                .error("NACKSLAK", &format!("Invalid Slack webhook URL: {}", e))
                .await;
            return;
        }
    };

    let host = match uri.host() {
        Some(h) => h.to_string(),
        None => {
            logger
                .error("NACKSLAK", "Slack webhook URL missing host")
                .await;
            return;
        }
    };

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_only()
        .enable_http1()
        .build();

    let client: hyper::Client<_, Body> = hyper::Client::builder().build(https);

    let req = match Request::builder()
        .method(Method::POST)
        .uri(webhook_url)
        .header("Content-Type", "application/json")
        .header("Host", &host)
        .body(Body::from(payload))
    {
        Ok(r) => r,
        Err(e) => {
            logger
                .error("NACKSLAK", &format!("Failed to build Slack request: {}", e))
                .await;
            return;
        }
    };

    match tokio::time::timeout(tokio::time::Duration::from_secs(10), client.request(req)).await {
        Ok(Ok(resp)) => {
            let status = resp.status();
            if !status.is_success() {
                logger
                    .warn(
                        "NACKSLAK",
                        &format!("Slack returned HTTP {}", status.as_u16()),
                    )
                    .await;
            }
        }
        Ok(Err(e)) => {
            logger
                .warn("NACKSLAK", &format!("Slack request failed: {}", e))
                .await;
        }
        Err(_) => {
            logger
                .warn("NACKSLAK", "Slack request timed out (10s)")
                .await;
        }
    }
}

// --- Monitor scheduler ---

async fn run_scheduler(state: Arc<AppState>) {
    // Tick every 60 seconds
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

    loop {
        interval.tick().await;
        let now = Utc::now();

        for (code, monitor) in &state.monitors {
            let cron = match Cron::new(&monitor.cron_expr).parse() {
                Ok(c) => c,
                Err(e) => {
                    state
                        .logger
                        .error(
                            "NACKCRON",
                            &format!("Bad cron expr for code={}: {}", code, e),
                        )
                        .await;
                    continue;
                }
            };

            let mut states = state.monitor_states.lock().await;
            let ms = states.entry(code.clone()).or_insert(MonitorState {
                alert_count: 0,
                last_checkin: None,
                last_alert: None,
            });

            // Find the most recent cron time whose grace period has expired.
            let grace = chrono::Duration::minutes(monitor.grace_min as i64);
            let lookback = now - chrono::Duration::hours(24);
            let last_scheduled = match find_last_scheduled(&cron, lookback, now - grace) {
                Some(t) => t,
                None => continue,
            };

            // Only alert for deadlines that passed after this instance started.
            if last_scheduled + grace < state.start_time {
                continue;
            }

            // Did a valid checkin arrive since the scheduled time?
            let checkin_ok = ms
                .last_checkin
                .map(|lc| lc >= last_scheduled)
                .unwrap_or(false);

            if checkin_ok {
                ms.alert_count = 0;
                continue;
            }

            // We have a miss. First alert fires immediately.
            // Subsequent alerts (escalations) throttle by escalation_grace_min.
            let should_alert = match ms.last_alert {
                None => true,
                Some(la) => {
                    let throttle = if ms.alert_count == 0 {
                        grace
                    } else {
                        chrono::Duration::minutes(monitor.escalation_grace_min as i64)
                    };
                    now - la >= throttle
                }
            };

            if !should_alert {
                continue;
            }

            ms.alert_count += 1;
            ms.last_alert = Some(now);

            let is_escalated = ms.alert_count > 1;
            let label = if is_escalated { "ESCALATION" } else { "MISS" };
            let log_code = if is_escalated { "NACKESCL" } else { "NACKMISS" };

            state
                .logger
                .warn(
                    log_code,
                    &format!(
                        "{}: code={} name={} alert_count={} scheduled={}",
                        label, code, monitor.name, ms.alert_count,
                        last_scheduled.format("%Y-%m-%dT%H:%M:%SZ")
                    ),
                )
                .await;

            if let Some(ref url) = monitor.slack_webhook {
                let suffix = monitor
                    .slack_message
                    .as_deref()
                    .map(|m| format!(" {}", m))
                    .unwrap_or_default();
                let slack_msg = if is_escalated {
                    format!(
                        ":rotating_light: ESCALATION: [{}] `{}` (code={}) missed check-in (alert #{}). {}{}",
                        state.config.server_name, monitor.name, code, ms.alert_count, monitor.description, suffix
                    )
                } else {
                    format!(
                        ":warning: [{}] `{}` (code={}) missed check-in. {}{}",
                        state.config.server_name, monitor.name, code, monitor.description, suffix
                    )
                };
                send_slack(url, &slack_msg, &state.logger).await;
            }
        }
    }
}

/// Walk backwards from `end` to find the most recent cron match at or after `start`.
fn find_last_scheduled(cron: &Cron, start: DateTime<Utc>, end: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut last = None;
    // Iterate forward from start, collect the last one before end
    let iter = cron.clone().iter_from(start);
    for t in iter {
        if t > end {
            break;
        }
        last = Some(t);
    }
    last
}

// --- Config loading ---

fn load_monitors(
    monitors_dir: &str,
) -> HashMap<String, MonitorConfig> {
    let mut monitors = HashMap::new();

    let entries = match fs::read_dir(monitors_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read monitors dir {}: {}", monitors_dir, e);
            return monitors;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let filename = match path.file_name().and_then(|f| f.to_str()) {
            Some(f) if f.ends_with(".env") => f.to_string(),
            _ => continue,
        };

        // Extract code: everything after last '-' before '.env'
        let stem = filename.strip_suffix(".env").unwrap();
        let (name_part, code) = match stem.rsplit_once('-') {
            Some((n, c)) => (n.to_string(), c.to_string()),
            None => {
                eprintln!(
                    "Skipping monitor file {} — expected format: name-CODE.env",
                    filename
                );
                continue;
            }
        };

        if code.len() < 4 || code.len() > 25 {
            eprintln!(
                "Skipping monitor file {} — code must be 4-25 characters",
                filename
            );
            continue;
        }

        let env_vars = load_config(path.to_str().unwrap());

        let cron_expr = match env_vars.get("NACK_CRON") {
            Some(c) => c.clone(),
            None => {
                eprintln!("Skipping monitor {} — missing NACK_CRON", filename);
                continue;
            }
        };

        // Validate cron expression at load time
        if let Err(e) = Cron::new(&cron_expr).parse() {
            eprintln!(
                "Skipping monitor {} — invalid NACK_CRON '{}': {}",
                filename, cron_expr, e
            );
            continue;
        }

        let description = env_vars
            .get("NACK_DESCRIPTION")
            .cloned()
            .unwrap_or_else(|| name_part.clone());

        let grace_min: u64 = env_vars
            .get("NACK_GRACE_MIN")
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);

        let slack_webhook = env_vars
            .get("NACK_SLACK_WEBHOOK")
            .cloned()
            .filter(|s| !s.is_empty());

        let slack_message = env_vars
            .get("NACK_SLACK_MESSAGE")
            .cloned()
            .filter(|s| !s.is_empty());

        let escalation_grace_min: u64 = match env_vars
            .get("NACK_ESCALATION_GRACE_MIN")
            .and_then(|v| v.parse().ok())
        {
            Some(v) => v,
            None => {
                eprintln!("Skipping monitor {} — missing or invalid NACK_ESCALATION_GRACE_MIN", filename);
                continue;
            }
        };

        if slack_webhook.is_none() {
            println!(
                "  NACK_SLACK_WEBHOOK not defined; slack disabled for code={}",
                code
            );
        }

        println!(
            "  Loaded monitor: {} (code={}, cron={}, grace={}min, escalation={}min)",
            name_part, code, cron_expr, grace_min, escalation_grace_min
        );

        monitors.insert(
            code.clone(),
            MonitorConfig {
                code,
                name: name_part,
                description,
                cron_expr,
                grace_min,
                slack_webhook,
                slack_message,
                escalation_grace_min,
            },
        );
    }

    monitors
}

// --- main ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("shrmpl-nackmon-srv version {}", VERSION);
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }

    let config = load_config(&args[1]);

    // Extract server config
    let listen_addr = config
        .get("NACK_LISTEN_ADDR")
        .expect("NACK_LISTEN_ADDR required")
        .clone();
    let listen_port: u16 = config
        .get("NACK_LISTEN_PORT")
        .expect("NACK_LISTEN_PORT required")
        .parse()
        .expect("NACK_LISTEN_PORT must be a valid port");
    let monitors_dir = config
        .get("NACK_MONITORS_DIR")
        .expect("NACK_MONITORS_DIR required")
        .clone();
    let status_path = config
        .get("NACK_STATUS_PATH")
        .expect("NACK_STATUS_PATH required")
        .clone();
    let status_path = if status_path.starts_with('/') {
        status_path
    } else {
        format!("/{}", status_path)
    };

    // Logger config
    let slog_dest = config
        .get("SLOG_DEST")
        .unwrap_or(&String::new())
        .clone();
    let slog_level = config
        .get("SLOG_LEVEL")
        .unwrap_or(&"INFO".to_string())
        .clone();
    let slog_console = config
        .get("SLOG_CONSOLE")
        .map(|s| s.parse().unwrap_or(true))
        .unwrap_or(true);
    let slog_send_actv = config
        .get("SLOG_SEND_ACTV")
        .map(|s| s.parse().unwrap_or(false))
        .unwrap_or(false);
    let slog_send_log = config
        .get("SLOG_SEND_LOG")
        .map(|s| s.parse().unwrap_or(true))
        .unwrap_or(true);

    let server_name = config
        .get("NACK_SERVER_NAME")
        .cloned()
        .unwrap_or_else(|| "shrmpl-nackmon".to_string());

    let logger = Logger::new(
        slog_dest,
        server_name.clone(),
        shrmpl::shrmpl_log_client::LogLevel::from_str(&slog_level),
        slog_console,
        slog_send_actv,
        slog_send_log,
    );
    logger.check_connectivity().await;

    // Load monitors
    println!("Loading monitors from {}...", monitors_dir);
    let monitors = load_monitors(&monitors_dir);

    if monitors.is_empty() {
        eprintln!("Warning: no monitors loaded from {}", monitors_dir);
    }

    let server_config = ServerConfig {
        listen_addr: listen_addr.clone(),
        listen_port,
        monitors_dir,
        status_path,
        server_name,
    };

    let state = Arc::new(AppState {
        config: server_config,
        monitors,
        monitor_states: Mutex::new(HashMap::new()),
        start_time: Utc::now(),
        logger: logger.clone(),
    });

    // Start the scheduler in the background
    let scheduler_state = state.clone();
    tokio::spawn(async move {
        run_scheduler(scheduler_state).await;
    });

    let addr: SocketAddr = format!("{}:{}", listen_addr, listen_port).parse()?;

    let start_msg = format!(
        "shrmpl-nackmon-srv version {} listening on {} (monitors={})",
        VERSION,
        addr,
        state.monitors.len()
    );
    println!("{}", start_msg);
    logger.info("NACKSTRT", &start_msg).await;

    // Log each loaded monitor
    for (code, monitor) in &state.monitors {
        logger
            .info(
                "NACKLOAD",
                &format!(
                    "Monitor loaded: code={} name={} cron={} grace={}min escalation={}min",
                    code, monitor.name, monitor.cron_expr, monitor.grace_min, monitor.escalation_grace_min
                ),
            )
            .await;
    }

    let listener = TcpListener::bind(&addr).await?;

    let make_svc = make_service_fn(move |_conn| {
        let state = state.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(req, state.clone())
            }))
        }
    });

    let server = Server::builder(hyper::server::accept::from_stream(
        async_stream::stream! {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        yield Ok::<_, hyper::Error>(stream);
                    }
                    Err(e) => {
                        eprintln!("Failed to accept connection: {}", e);
                    }
                }
            }
        },
    ))
    .serve(make_svc);

    logger.info("NACKSTRT", "HTTP server started").await;

    if let Err(e) = server.await {
        logger
            .error("NACKSHUT", &format!("Server error: {}", e))
            .await;
    }

    Ok(())
}
