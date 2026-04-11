const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
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
    endpoints_dir: String,
    status_path: String,
    server_name: String,
    admin_slack_webhook: Option<String>,
}

#[derive(Clone)]
#[allow(dead_code)]
struct EndpointConfig {
    code: String,
    name: String,
    url: String,
    check_interval_sec: u64,
    expect_status: u16,
    slack_webhook: Option<String>,
    slack_message: Option<String>,
    escalation_min: u64,
}

#[derive(Clone)]
struct EndpointState {
    last_check: Option<DateTime<Utc>>,
    last_status: Option<u16>,
    last_error: Option<String>,
    is_healthy: bool,
    alert_count: u32,
    last_alert: Option<DateTime<Utc>>,
    cert_expiry: Option<DateTime<Utc>>,
    cert_alert_sent_date: Option<String>,
}

impl EndpointState {
    fn new() -> Self {
        Self {
            last_check: None,
            last_status: None,
            last_error: None,
            is_healthy: true,
            alert_count: 0,
            last_alert: None,
            cert_expiry: None,
            cert_alert_sent_date: None,
        }
    }
}

#[allow(dead_code)]
struct AppState {
    config: ServerConfig,
    endpoints: HashMap<String, EndpointConfig>,
    endpoint_states: Mutex<HashMap<String, EndpointState>>,
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

    match (&method, path.as_str()) {
        (&Method::GET, "/health") => handle_health(&state).await,
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
        r#"{{"status":"ok","endpoints_loaded":{},"uptime_seconds":{}}}"#,
        state.endpoints.len(),
        uptime
    );
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

async fn handle_status(state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let states = state.endpoint_states.lock().await;
    let mut entries = Vec::new();

    for (code, endpoint) in &state.endpoints {
        let es = states.get(code);

        let last_check = es
            .and_then(|s| s.last_check)
            .map(|t| format!("\"{}\"", t.format("%Y-%m-%dT%H:%M:%SZ")))
            .unwrap_or_else(|| "null".to_string());

        let last_status = es
            .and_then(|s| s.last_status)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "null".to_string());

        let last_error = es
            .and_then(|s| s.last_error.as_ref())
            .map(|e| format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\"")))
            .unwrap_or_else(|| "null".to_string());

        let is_healthy = es.map(|s| s.is_healthy).unwrap_or(true);
        let alert_count = es.map(|s| s.alert_count).unwrap_or(0);

        let cert_expiry = es
            .and_then(|s| s.cert_expiry)
            .map(|t| format!("\"{}\"", t.format("%Y-%m-%dT%H:%M:%SZ")))
            .unwrap_or_else(|| "null".to_string());

        entries.push(format!(
            r#"{{"code":"{}","name":"{}","url":"{}","is_healthy":{},"last_check":{},"last_status":{},"last_error":{},"alert_count":{},"cert_expiry":{}}}"#,
            code,
            endpoint.name,
            endpoint.url.replace('\\', "\\\\").replace('"', "\\\""),
            is_healthy,
            last_check,
            last_status,
            last_error,
            alert_count,
            cert_expiry
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
                .error("PULSSLAK", &format!("Invalid Slack webhook URL: {}", e))
                .await;
            return;
        }
    };

    let host = match uri.host() {
        Some(h) => h.to_string(),
        None => {
            logger
                .error("PULSSLAK", "Slack webhook URL missing host")
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
                .error("PULSSLAK", &format!("Failed to build Slack request: {}", e))
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
                        "PULSSLAK",
                        &format!("Slack returned HTTP {}", status.as_u16()),
                    )
                    .await;
            }
        }
        Ok(Err(e)) => {
            logger
                .warn("PULSSLAK", &format!("Slack request failed: {}", e))
                .await;
        }
        Err(_) => {
            logger
                .warn("PULSSLAK", "Slack request timed out (10s)")
                .await;
        }
    }
}

// --- Endpoint health checking ---

async fn check_endpoint(url: &str, expect_status: u16) -> (bool, Option<u16>, Option<String>) {
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_or_http()
        .enable_http1()
        .build();

    let client: hyper::Client<_, Body> = hyper::Client::builder().build(https);

    let uri: hyper::Uri = match url.parse() {
        Ok(u) => u,
        Err(e) => {
            return (false, None, Some(format!("invalid URL: {}", e)));
        }
    };

    let host = uri.host().unwrap_or("").to_string();

    let req = match Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("Host", &host)
        .body(Body::empty())
    {
        Ok(r) => r,
        Err(e) => {
            return (false, None, Some(format!("request build error: {}", e)));
        }
    };

    match tokio::time::timeout(tokio::time::Duration::from_secs(10), client.request(req)).await {
        Ok(Ok(resp)) => {
            let status = resp.status().as_u16();
            if status == expect_status {
                (true, Some(status), None)
            } else {
                (
                    false,
                    Some(status),
                    Some(format!("expected {} got {}", expect_status, status)),
                )
            }
        }
        Ok(Err(e)) => (false, None, Some(format!("connection error: {}", e))),
        Err(_) => (false, None, Some("request timed out (10s)".to_string())),
    }
}

async fn run_endpoint_checker(code: String, state: Arc<AppState>) {
    let endpoint = match state.endpoints.get(&code) {
        Some(e) => e.clone(),
        None => return,
    };

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(endpoint.check_interval_sec));

    loop {
        interval.tick().await;

        let (is_healthy, status, error) =
            check_endpoint(&endpoint.url, endpoint.expect_status).await;
        let now = Utc::now();

        let mut states = state.endpoint_states.lock().await;
        let es = states
            .entry(code.clone())
            .or_insert_with(EndpointState::new);

        let was_healthy = es.is_healthy;

        es.last_check = Some(now);
        es.last_status = status;
        es.last_error = error.clone();
        es.is_healthy = is_healthy;

        if is_healthy {
            // Recovery: was down, now up
            if !was_healthy && es.alert_count > 0 {
                let prev_alert_count = es.alert_count;
                es.alert_count = 0;
                es.last_alert = None;

                let suffix = endpoint
                    .slack_message
                    .as_deref()
                    .map(|m| format!(" {}", m))
                    .unwrap_or_default();

                state
                    .logger
                    .info(
                        "PULSCHEK",
                        &format!(
                            "RECOVERED: code={} name={} url={} (was alert #{})",
                            code, endpoint.name, endpoint.url, prev_alert_count
                        ),
                    )
                    .await;

                if let Some(ref url) = endpoint.slack_webhook {
                    let msg = format!(
                        ":white_check_mark: [{}] `{}` (code={}) is UP — alarm cleared.{}",
                        state.config.server_name, endpoint.name, code, suffix
                    );
                    // Drop lock before sending Slack
                    drop(states);
                    send_slack(url, &msg, &state.logger).await;
                }
            } else {
                state
                    .logger
                    .debug(
                        "PULSCHEK",
                        &format!("OK: code={} name={} url={}", code, endpoint.name, endpoint.url),
                    )
                    .await;
            }
        } else {
            let error_desc = error.unwrap_or_else(|| "unknown error".to_string());

            if was_healthy || es.alert_count == 0 {
                // First failure
                es.alert_count = 1;
                es.last_alert = Some(now);

                let suffix = endpoint
                    .slack_message
                    .as_deref()
                    .map(|m| format!(" {}", m))
                    .unwrap_or_default();

                state
                    .logger
                    .warn(
                        "PULSCHEK",
                        &format!(
                            "DOWN: code={} name={} url={} error={}",
                            code, endpoint.name, endpoint.url, error_desc
                        ),
                    )
                    .await;

                if let Some(ref url) = endpoint.slack_webhook {
                    let msg = format!(
                        ":warning: [{}] `{}` (code={}) is DOWN — {}.{}",
                        state.config.server_name, endpoint.name, code, error_desc, suffix
                    );
                    drop(states);
                    send_slack(url, &msg, &state.logger).await;
                }
            } else {
                // Still down — check escalation
                let should_escalate = match es.last_alert {
                    Some(la) => {
                        let elapsed = now - la;
                        elapsed >= chrono::Duration::minutes(endpoint.escalation_min as i64)
                    }
                    None => true,
                };

                if should_escalate {
                    es.alert_count += 1;
                    es.last_alert = Some(now);
                    let alert_num = es.alert_count;

                    let suffix = endpoint
                        .slack_message
                        .as_deref()
                        .map(|m| format!(" {}", m))
                        .unwrap_or_default();

                    state
                        .logger
                        .warn(
                            "PULSCHEK",
                            &format!(
                                "ESCALATION: code={} name={} alert_count={} url={} error={}",
                                code, endpoint.name, alert_num, endpoint.url, error_desc
                            ),
                        )
                        .await;

                    if let Some(ref url) = endpoint.slack_webhook {
                        let msg = format!(
                            ":rotating_light: ESCALATION: [{}] `{}` (code={}) still DOWN (alert #{}).{}",
                            state.config.server_name, endpoint.name, code, alert_num, suffix
                        );
                        drop(states);
                        send_slack(url, &msg, &state.logger).await;
                    }
                }
            }
        }
    }
}

// --- SSL certificate checking ---

async fn check_cert(host: &str, port: u16) -> Result<DateTime<Utc>, String> {
    use rustls::{ClientConfig, Certificate, RootCertStore, ServerName};
    use tokio_rustls::TlsConnector;
    use x509_parser::prelude::*;

    let mut root_store = RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().map_err(|e| format!("load certs: {}", e))?
    {
        let _ = root_store.add(&Certificate(cert.0));
    }

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));
    let addr = format!("{}:{}", host, port);

    let stream = match tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("TCP connect to {}: {}", addr, e)),
        Err(_) => return Err(format!("TCP connect to {} timed out", addr)),
    };

    let server_name =
        ServerName::try_from(host).map_err(|e| format!("invalid server name '{}': {}", host, e))?;

    let tls_stream = match tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        connector.connect(server_name, stream),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("TLS handshake with {}: {}", host, e)),
        Err(_) => return Err(format!("TLS handshake with {} timed out", host)),
    };

    let (_, conn) = tls_stream.get_ref();
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| "no peer certificates".to_string())?;

    let leaf = certs
        .first()
        .ok_or_else(|| "empty certificate chain".to_string())?;

    let (_, cert) = X509Certificate::from_der(&leaf.0)
        .map_err(|e| format!("x509 parse error: {}", e))?;

    let not_after = cert.validity().not_after;
    let ts = not_after.timestamp();
    let expiry = DateTime::from_timestamp(ts, 0)
        .ok_or_else(|| "invalid timestamp in certificate".to_string())?;

    Ok(expiry)
}

async fn run_cert_checker(state: Arc<AppState>) {
    // Run immediately on startup, then daily at midnight UTC
    let mut first_run = true;

    loop {
        if !first_run {
            // Sleep until next midnight UTC
            let now = Utc::now();
            let tomorrow = (now + chrono::Duration::days(1))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let tomorrow_utc: DateTime<Utc> =
                DateTime::from_naive_utc_and_offset(tomorrow, Utc);
            let sleep_dur = (tomorrow_utc - now).to_std().unwrap_or(std::time::Duration::from_secs(3600));
            state
                .logger
                .info(
                    "PULSCERT",
                    &format!(
                        "Next cert check at midnight UTC (sleeping {}s)",
                        sleep_dur.as_secs()
                    ),
                )
                .await;
            tokio::time::sleep(sleep_dur).await;
        }
        first_run = false;

        let today = Utc::now().format("%Y-%m-%d").to_string();

        for (code, endpoint) in &state.endpoints {
            if !endpoint.url.starts_with("https://") {
                continue;
            }

            // Parse host and port from URL
            let uri: hyper::Uri = match endpoint.url.parse() {
                Ok(u) => u,
                Err(_) => continue,
            };
            let host = match uri.host() {
                Some(h) => h.to_string(),
                None => continue,
            };
            let port = uri.port_u16().unwrap_or(443);

            match check_cert(&host, port).await {
                Ok(expiry) => {
                    let now = Utc::now();
                    let days_remaining = (expiry - now).num_days();

                    let mut states = state.endpoint_states.lock().await;
                    let es = states.entry(code.clone()).or_insert_with(EndpointState::new);
                    es.cert_expiry = Some(expiry);

                    state
                        .logger
                        .info(
                            "PULSCERT",
                            &format!(
                                "Cert check: code={} host={} expires={} days_remaining={}",
                                code,
                                host,
                                expiry.format("%Y-%m-%d"),
                                days_remaining
                            ),
                        )
                        .await;

                    if days_remaining <= 10 {
                        let already_sent = es
                            .cert_alert_sent_date
                            .as_ref()
                            .map(|d| d == &today)
                            .unwrap_or(false);

                        if !already_sent {
                            es.cert_alert_sent_date = Some(today.clone());

                            if let Some(ref webhook) = endpoint.slack_webhook {
                                let suffix = endpoint
                                    .slack_message
                                    .as_deref()
                                    .map(|m| format!(" {}", m))
                                    .unwrap_or_default();
                                let msg = format!(
                                    ":lock: CERT WARNING: [{}] `{}` (code={}) SSL cert for {} expires {} ({}d remaining).{}",
                                    state.config.server_name,
                                    endpoint.name,
                                    code,
                                    host,
                                    expiry.format("%Y-%m-%d"),
                                    days_remaining,
                                    suffix
                                );
                                drop(states);
                                send_slack(webhook, &msg, &state.logger).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    state
                        .logger
                        .warn(
                            "PULSCERT",
                            &format!("Cert check failed: code={} host={} error={}", code, host, e),
                        )
                        .await;
                }
            }
        }
    }
}

// --- Config loading ---

fn load_endpoints(endpoints_dir: &str) -> (HashMap<String, EndpointConfig>, Vec<(String, String)>) {
    let mut endpoints = HashMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();

    let entries = match fs::read_dir(endpoints_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read endpoints dir {}: {}", endpoints_dir, e);
            return (endpoints, skipped);
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

        let stem = filename.strip_suffix(".env").unwrap();
        let (name_part, code) = match stem.rsplit_once('-') {
            Some((n, c)) => (n.to_string(), c.to_string()),
            None => {
                let reason = "expected format: name-CODE.env".to_string();
                eprintln!("Skipping endpoint file {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
        };

        if code.len() < 4 || code.len() > 25 {
            let reason = "code must be 4-25 characters".to_string();
            eprintln!("Skipping endpoint file {} — {}", filename, reason);
            skipped.push((filename, reason));
            continue;
        }

        let env_vars = load_config(path.to_str().unwrap());

        let url = match env_vars.get("PULSE_URL") {
            Some(u) if u.starts_with("http://") || u.starts_with("https://") => u.clone(),
            Some(u) => {
                let reason = format!("PULSE_URL must start with http:// or https://, got '{}'", u);
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
            None => {
                let reason = "missing PULSE_URL".to_string();
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
        };

        let check_interval_sec: u64 = match env_vars
            .get("PULSE_INTERVAL_SEC")
            .and_then(|v| v.parse().ok())
        {
            Some(v) if v >= 5 => v,
            Some(v) => {
                let reason = format!("PULSE_INTERVAL_SEC must be >= 5, got {}", v);
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
            None => {
                let reason = "missing or invalid PULSE_INTERVAL_SEC".to_string();
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
        };

        let expect_status: u16 = env_vars
            .get("PULSE_EXPECT_STATUS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);

        if expect_status < 100 || expect_status > 999 {
            let reason = format!("PULSE_EXPECT_STATUS must be 100-999, got {}", expect_status);
            eprintln!("Skipping endpoint {} — {}", filename, reason);
            skipped.push((filename, reason));
            continue;
        }

        let escalation_min: u64 = match env_vars
            .get("PULSE_ESCALATION_MIN")
            .and_then(|v| v.parse().ok())
        {
            Some(v) if v >= 1 => v,
            Some(v) => {
                let reason = format!("PULSE_ESCALATION_MIN must be >= 1, got {}", v);
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
            None => {
                let reason = "missing or invalid PULSE_ESCALATION_MIN".to_string();
                eprintln!("Skipping endpoint {} — {}", filename, reason);
                skipped.push((filename, reason));
                continue;
            }
        };

        let slack_webhook = env_vars
            .get("PULSE_SLACK_WEBHOOK")
            .cloned()
            .filter(|s| !s.is_empty());

        let slack_message = env_vars
            .get("PULSE_SLACK_MESSAGE")
            .cloned()
            .filter(|s| !s.is_empty());

        if slack_webhook.is_none() {
            println!(
                "  PULSE_SLACK_WEBHOOK not defined; slack disabled for code={}",
                code
            );
        }

        println!(
            "  Loaded endpoint: {} (code={}, url={}, interval={}s, expect={}, escalation={}min)",
            name_part, code, url, check_interval_sec, expect_status, escalation_min
        );

        endpoints.insert(
            code.clone(),
            EndpointConfig {
                code,
                name: name_part,
                url,
                check_interval_sec,
                expect_status,
                slack_webhook,
                slack_message,
                escalation_min,
            },
        );
    }

    (endpoints, skipped)
}

// --- main ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("shrmpl-pulsecheck-srv version {}", VERSION);
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }

    let config = load_config(&args[1]);

    // Extract server config
    let listen_addr = config
        .get("PULSE_LISTEN_ADDR")
        .expect("PULSE_LISTEN_ADDR required")
        .clone();
    let listen_port: u16 = config
        .get("PULSE_LISTEN_PORT")
        .expect("PULSE_LISTEN_PORT required")
        .parse()
        .expect("PULSE_LISTEN_PORT must be a valid port");
    let endpoints_dir = config
        .get("PULSE_ENDPOINTS_DIR")
        .expect("PULSE_ENDPOINTS_DIR required")
        .clone();
    let status_path = config
        .get("PULSE_STATUS_PATH")
        .expect("PULSE_STATUS_PATH required")
        .clone();
    let status_path = if status_path.starts_with('/') {
        status_path
    } else {
        format!("/{}", status_path)
    };

    let server_name = config
        .get("PULSE_SERVER_NAME")
        .cloned()
        .unwrap_or_else(|| "shrmpl-pulsecheck".to_string());

    let admin_slack_webhook = config
        .get("PULSE_ADMIN_SLACK_WEBHOOK")
        .cloned()
        .filter(|s| !s.is_empty());

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

    let logger = Logger::new(
        slog_dest,
        server_name.clone(),
        shrmpl::shrmpl_log_client::LogLevel::from_str(&slog_level),
        slog_console,
        slog_send_actv,
        slog_send_log,
    );
    logger.check_connectivity().await;

    // Load endpoints
    println!("Loading endpoints from {}...", endpoints_dir);
    let (endpoints, skipped) = load_endpoints(&endpoints_dir);

    if endpoints.is_empty() {
        eprintln!("Warning: no endpoints loaded from {}", endpoints_dir);
    }

    // Send Slack notification for any skipped config files
    if !skipped.is_empty() {
        if let Some(ref admin_webhook) = admin_slack_webhook {
            let skip_lines: Vec<String> = skipped
                .iter()
                .map(|(file, reason)| format!("• `{}`: {}", file, reason))
                .collect();
            let msg = format!(
                ":no_entry: [{}] Skipped {} endpoint file(s) on startup:\n{}",
                server_name,
                skipped.len(),
                skip_lines.join("\n")
            );
            send_slack(admin_webhook, &msg, &logger).await;
        }
    }

    let server_config = ServerConfig {
        listen_addr: listen_addr.clone(),
        listen_port,
        endpoints_dir,
        status_path,
        server_name,
        admin_slack_webhook,
    };

    let state = Arc::new(AppState {
        config: server_config,
        endpoints,
        endpoint_states: Mutex::new(HashMap::new()),
        start_time: Utc::now(),
        logger: logger.clone(),
    });

    // Spawn per-endpoint health checker tasks
    for (code, _endpoint) in &state.endpoints {
        let s = state.clone();
        let c = code.clone();
        tokio::spawn(async move {
            run_endpoint_checker(c, s).await;
        });
    }

    // Spawn SSL certificate checker task
    let cert_state = state.clone();
    tokio::spawn(async move {
        run_cert_checker(cert_state).await;
    });

    let addr: SocketAddr = format!("{}:{}", listen_addr, listen_port).parse()?;

    let start_msg = format!(
        "shrmpl-pulsecheck-srv version {} listening on {} (endpoints={})",
        VERSION,
        addr,
        state.endpoints.len()
    );
    println!("{}", start_msg);
    logger.info("PULSSTRT", &start_msg).await;

    // Log each loaded endpoint
    for (code, endpoint) in &state.endpoints {
        logger
            .info(
                "PULSLOAD",
                &format!(
                    "Endpoint loaded: code={} name={} url={} interval={}s expect={} escalation={}min",
                    code, endpoint.name, endpoint.url, endpoint.check_interval_sec,
                    endpoint.expect_status, endpoint.escalation_min
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

    logger.info("PULSHTTP", "HTTP server started").await;

    if let Err(e) = server.await {
        logger
            .error("PULSHTTP", &format!("Server error: {}", e))
            .await;
    }

    Ok(())
}
