const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use hyper::body::HttpBody;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use shrmpl::config::load_config;
use shrmpl::shrmpl_log_client::Logger;

type HmacSha256 = Hmac<Sha256>;

// --- Data types ---

#[allow(dead_code)]
struct ServerConfig {
    tls_mode: String,
    listen_addr: String,
    listen_port: u16,
    tls_cert: String,
    tls_key: String,
    hooks_dir: String,
    max_concurrent: usize,
    default_timeout: u64,
}

#[derive(Clone)]
struct HookConfig {
    guid: String,
    provider: String,
    secret: String,
    script: String,
    timeout: u64,
    dedupe_window: usize,
    slack_webhook: Option<String>,
    env_vars: HashMap<String, String>,
}

#[derive(Clone)]
struct LastRunInfo {
    delivery_id: String,
    started_at: String,
    finished_at: String,
    exit_code: i32,
    duration_seconds: u64,
}

struct AppState {
    hooks: HashMap<String, HookConfig>,
    run_locks: Mutex<HashSet<String>>,
    dedupe_buffers: Mutex<HashMap<String, VecDeque<String>>>,
    active_count: AtomicUsize,
    config: ServerConfig,
    start_time: Instant,
    last_runs: Mutex<HashMap<String, LastRunInfo>>,
    logger: Logger,
}

struct WebhookInfo {
    delivery_id: String,
    branch: String,
    event: String,
    repo: String,
    commit: String,
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
        (&Method::GET, p) if p.starts_with("/status/") => {
            let guid = &p[8..];
            handle_status(guid, &state).await
        }
        (&Method::POST, p) if p.starts_with("/hook/") => {
            let guid = p[6..].to_string();
            handle_hook(req, guid, state).await
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap()),
    }
}

async fn handle_health(state: &AppState) -> Result<Response<Body>, hyper::Error> {
    let uptime = state.start_time.elapsed().as_secs();
    let active = state.active_count.load(Ordering::Relaxed);
    let body = format!(
        r#"{{"status":"ok","hooks_loaded":{},"active_runs":{},"uptime_seconds":{}}}"#,
        state.hooks.len(),
        active,
        uptime
    );
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

async fn handle_status(guid: &str, state: &AppState) -> Result<Response<Body>, hyper::Error> {
    if !state.hooks.contains_key(guid) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(r#"{"error":"unknown guid"}"#))
            .unwrap());
    }

    let locks = state.run_locks.lock().await;
    let running = locks.contains(guid);
    drop(locks);

    let hook_state = if running { "running" } else { "idle" };

    let last_runs = state.last_runs.lock().await;
    let last_run_json = match last_runs.get(guid) {
        Some(lr) => format!(
            r#"{{"delivery_id":"{}","started_at":"{}","finished_at":"{}","exit_code":{},"duration_seconds":{}}}"#,
            lr.delivery_id, lr.started_at, lr.finished_at, lr.exit_code, lr.duration_seconds
        ),
        None => "null".to_string(),
    };

    let body = format!(
        r#"{{"guid":"{}","state":"{}","last_run":{}}}"#,
        guid, hook_state, last_run_json
    );
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

async fn handle_hook(
    req: Request<Body>,
    guid: String,
    state: Arc<AppState>,
) -> Result<Response<Body>, hyper::Error> {
    let hook = match state.hooks.get(&guid) {
        Some(h) => h.clone(),
        None => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(r#"{"error":"unknown guid"}"#))
                .unwrap());
        }
    };

    state
        .logger
        .info("CICDRECV", &format!("Webhook received for guid={}", guid))
        .await;

    // Read body (capped at 1MB)
    let headers = req.headers().clone();
    let body_bytes = match read_body(req.into_body(), 1_048_576).await {
        Ok(b) => b,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .body(Body::from(r#"{"error":"body too large"}"#))
                .unwrap());
        }
    };

    // Validate
    let valid = match hook.provider.as_str() {
        "github" => validate_github(&body_bytes, &hook.secret, &headers),
        "azure-devops" => validate_azure_devops(&hook.secret, &headers),
        "generic" => validate_generic(&hook.secret, &headers),
        _ => false,
    };

    if !valid {
        state
            .logger
            .warn("CICDAUTH", &format!("Validation failed for guid={}", guid))
            .await;
        return Ok(json_200(r#"{"status":"rejected","reason":"auth_failed"}"#));
    }

    // Extract webhook info
    let info = extract_webhook_info(&hook.provider, &headers, &body_bytes);

    // Dedupe check
    {
        let mut dedupe = state.dedupe_buffers.lock().await;
        let buffer = dedupe
            .entry(guid.clone())
            .or_insert_with(|| VecDeque::with_capacity(hook.dedupe_window));
        if buffer.contains(&info.delivery_id) {
            state
                .logger
                .debug(
                    "CICDDUP",
                    &format!(
                        "Duplicate delivery_id={} for guid={}",
                        info.delivery_id, guid
                    ),
                )
                .await;
            return Ok(json_200(r#"{"status":"rejected","reason":"duplicate"}"#));
        }
        if buffer.len() >= hook.dedupe_window {
            buffer.pop_front();
        }
        buffer.push_back(info.delivery_id.clone());
    }

    // Max concurrent check
    let current = state.active_count.load(Ordering::Relaxed);
    if current >= state.config.max_concurrent {
        state
            .logger
            .warn(
                "CICDLIMIT",
                &format!("Max concurrent reached ({}) for guid={}", current, guid),
            )
            .await;
        return Ok(json_200(
            r#"{"status":"rejected","reason":"max_concurrent"}"#,
        ));
    }

    // Run lock check
    {
        let mut locks = state.run_locks.lock().await;
        if locks.contains(&guid) {
            state
                .logger
                .warn(
                    "CICDLOCK",
                    &format!("Already running for guid={}", guid),
                )
                .await;
            return Ok(json_200(
                r#"{"status":"rejected","reason":"already_running"}"#,
            ));
        }
        locks.insert(guid.clone());
    }

    state.active_count.fetch_add(1, Ordering::Relaxed);

    let response_body = format!(
        r#"{{"status":"accepted","guid":"{}","delivery_id":"{}"}}"#,
        guid, info.delivery_id
    );

    // Spawn script execution in background
    let state_bg = state.clone();
    let hook_bg = hook.clone();
    tokio::spawn(async move {
        run_script(&hook_bg, &info, &state_bg).await;
    });

    Ok(json_200(&response_body))
}

fn json_200(body: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn read_body(mut body: Body, max_size: usize) -> Result<Vec<u8>, &'static str> {
    let mut buf = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|_| "read error")?;
        if buf.len() + chunk.len() > max_size {
            return Err("body too large");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

// --- Webhook validation ---

fn validate_github(body: &[u8], secret: &str, headers: &hyper::HeaderMap) -> bool {
    let sig_header = match headers.get("x-hub-signature-256") {
        Some(v) => match v.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return false,
        },
        None => return false,
    };

    let expected = match sig_header.strip_prefix("sha256=") {
        Some(hex) => hex,
        None => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();
    let computed = hex_encode(&result);

    constant_time_eq(expected.as_bytes(), computed.as_bytes())
}

fn validate_azure_devops(secret: &str, headers: &hyper::HeaderMap) -> bool {
    if let Some(rest) = secret.strip_prefix("basic:") {
        // basic:user:pass — validate Authorization header
        if let Some((user, pass)) = rest.split_once(':') {
            let expected = base64_encode(&format!("{}:{}", user, pass));
            let auth_header = match headers.get("authorization") {
                Some(v) => match v.to_str() {
                    Ok(s) => s.to_string(),
                    Err(_) => return false,
                },
                None => return false,
            };
            let provided = match auth_header.strip_prefix("Basic ") {
                Some(b) => b,
                None => return false,
            };
            return constant_time_eq(expected.as_bytes(), provided.as_bytes());
        }
        return false;
    }

    if let Some(expected_value) = secret.strip_prefix("header:") {
        // header:value — match against X-Hook-Secret
        return match headers.get("x-hook-secret") {
            Some(v) => match v.to_str() {
                Ok(s) => constant_time_eq(expected_value.as_bytes(), s.as_bytes()),
                Err(_) => false,
            },
            None => false,
        };
    }

    false
}

fn validate_generic(secret: &str, headers: &hyper::HeaderMap) -> bool {
    match headers.get("x-hook-secret") {
        Some(v) => match v.to_str() {
            Ok(s) => constant_time_eq(secret.as_bytes(), s.as_bytes()),
            Err(_) => false,
        },
        None => false,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < bytes.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if i + 2 < bytes.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

// --- Payload extraction ---

fn extract_webhook_info(
    provider: &str,
    headers: &hyper::HeaderMap,
    body: &[u8],
) -> WebhookInfo {
    match provider {
        "github" => extract_github_info(headers, body),
        "azure-devops" => extract_azure_info(body),
        "generic" => extract_generic_info(headers, body),
        _ => WebhookInfo {
            delivery_id: sha256_hex(body),
            branch: String::new(),
            event: String::new(),
            repo: String::new(),
            commit: String::new(),
        },
    }
}

fn extract_github_info(headers: &hyper::HeaderMap, body: &[u8]) -> WebhookInfo {
    let delivery_id = header_str(headers, "x-github-delivery")
        .unwrap_or_else(|| sha256_hex(body));
    let event = header_str(headers, "x-github-event").unwrap_or_default();

    let body_str = String::from_utf8_lossy(body);
    let branch = json_extract_string(&body_str, "ref")
        .map(|r| r.strip_prefix("refs/heads/").unwrap_or(&r).to_string())
        .unwrap_or_default();
    let repo = json_extract_string(&body_str, "full_name").unwrap_or_default();
    let commit = json_extract_string(&body_str, "after").unwrap_or_default();

    WebhookInfo {
        delivery_id,
        branch,
        event,
        repo,
        commit,
    }
}

fn extract_azure_info(body: &[u8]) -> WebhookInfo {
    let body_str = String::from_utf8_lossy(body);
    let delivery_id = json_extract_string(&body_str, "id")
        .unwrap_or_else(|| sha256_hex(body));
    let event = json_extract_string(&body_str, "eventType").unwrap_or_default();

    WebhookInfo {
        delivery_id,
        branch: String::new(),
        event,
        repo: String::new(),
        commit: String::new(),
    }
}

fn extract_generic_info(headers: &hyper::HeaderMap, body: &[u8]) -> WebhookInfo {
    let delivery_id = header_str(headers, "x-delivery-id")
        .unwrap_or_else(|| sha256_hex(body));

    WebhookInfo {
        delivery_id,
        branch: String::new(),
        event: String::new(),
        repo: String::new(),
        commit: String::new(),
    }
}

fn header_str(headers: &hyper::HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(|s| s.to_string())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let hash = Sha256::digest(data);
    hex_encode(&hash)
}

/// Minimal JSON string extractor — finds `"key":"value"` in flat JSON.
/// Not a full parser; sufficient for webhook payloads.
fn json_extract_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{}""#, key);
    let idx = json.find(&pattern)?;
    let after_key = &json[idx + pattern.len()..];
    // skip optional whitespace and colon
    let after_colon = after_key.trim_start();
    let after_colon = after_colon.strip_prefix(':')?;
    let after_colon = after_colon.trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let value_start = &after_colon[1..];
    let mut result = String::new();
    let mut chars = value_start.chars();
    loop {
        match chars.next() {
            Some('\\') => {
                if let Some(c) = chars.next() {
                    result.push(c);
                }
            }
            Some('"') => break,
            Some(c) => result.push(c),
            None => break,
        }
    }
    Some(result)
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
                .error("CICDSLAK", &format!("Invalid Slack webhook URL: {}", e))
                .await;
            return;
        }
    };

    let host = match uri.host() {
        Some(h) => h.to_string(),
        None => {
            logger
                .error("CICDSLAK", "Slack webhook URL missing host")
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
                .error("CICDSLAK", &format!("Failed to build Slack request: {}", e))
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
                        "CICDSLAK",
                        &format!("Slack returned HTTP {}", status.as_u16()),
                    )
                    .await;
            }
        }
        Ok(Err(e)) => {
            logger
                .warn("CICDSLAK", &format!("Slack request failed: {}", e))
                .await;
        }
        Err(_) => {
            logger.warn("CICDSLAK", "Slack request timed out (10s)").await;
        }
    }
}

// --- Script execution ---

async fn run_script(hook: &HookConfig, info: &WebhookInfo, state: &AppState) {
    let now = chrono::Utc::now();
    let started_at = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    state
        .logger
        .info(
            "CICDRUN",
            &format!(
                "Starting script for guid={} delivery_id={} script={}",
                hook.guid, info.delivery_id, hook.script
            ),
        )
        .await;

    // Slack: pipeline started
    if let Some(ref url) = hook.slack_webhook {
        let branch_info = if info.branch.is_empty() {
            String::new()
        } else {
            format!(" branch={}", info.branch)
        };
        let msg = format!(
            ":rocket: Pipeline started for `{}`{} (delivery={})",
            hook.guid, branch_info, info.delivery_id
        );
        send_slack(url, &msg, &state.logger).await;
    }

    let mut cmd = tokio::process::Command::new(&hook.script);
    cmd.env("SHRMPL_HOOK_GUID", &hook.guid);
    cmd.env("SHRMPL_DELIVERY_ID", &info.delivery_id);
    cmd.env("SHRMPL_TRIGGER_BRANCH", &info.branch);
    cmd.env("SHRMPL_TRIGGER_EVENT", &info.event);
    cmd.env("SHRMPL_TRIGGER_REPO", &info.repo);
    cmd.env("SHRMPL_TRIGGER_COMMIT", &info.commit);
    cmd.env("SHRMPL_TRIGGER_TIMESTAMP", &started_at);

    // Inject HOOK_* env vars from hook config
    for (k, v) in &hook.env_vars {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let timeout_dur = tokio::time::Duration::from_secs(hook.timeout);

    let result = match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let logger_out = state.logger.clone();
            let guid_out = hook.guid.clone();
            let stdout_task = tokio::spawn(async move {
                if let Some(stdout) = stdout {
                    let reader = TokioBufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        logger_out
                            .debug("CICDOUT", &format!("[stdout] guid={} {}", guid_out, line))
                            .await;
                    }
                }
            });

            let logger_err = state.logger.clone();
            let guid_err = hook.guid.clone();
            let stderr_task = tokio::spawn(async move {
                if let Some(stderr) = stderr {
                    let reader = TokioBufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        logger_err
                            .debug("CICDOUT", &format!("[stderr] guid={} {}", guid_err, line))
                            .await;
                    }
                }
            });

            tokio::select! {
                status = child.wait() => {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    match status {
                        Ok(s) => s.code().unwrap_or(-1),
                        Err(e) => {
                            state.logger.error("CICDFAIL", &format!("Wait failed for guid={}: {}", hook.guid, e)).await;
                            -1
                        }
                    }
                }
                _ = tokio::time::sleep(timeout_dur) => {
                    let _ = child.kill().await;
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    state.logger.error("CICDFAIL", &format!("Timeout ({}s) for guid={}", hook.timeout, hook.guid)).await;
                    -1
                }
            }
        }
        Err(e) => {
            state
                .logger
                .error(
                    "CICDFAIL",
                    &format!("Failed to spawn script for guid={}: {}", hook.guid, e),
                )
                .await;
            -1
        }
    };

    let finished_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let duration = (chrono::Utc::now() - now).num_seconds().unsigned_abs();

    if result == 0 {
        state
            .logger
            .info(
                "CICDDONE",
                &format!(
                    "Script completed for guid={} exit_code={} duration={}s",
                    hook.guid, result, duration
                ),
            )
            .await;
    } else {
        state
            .logger
            .error(
                "CICDFAIL",
                &format!(
                    "Script failed for guid={} exit_code={} duration={}s",
                    hook.guid, result, duration
                ),
            )
            .await;
    }

    // Slack: pipeline result
    if let Some(ref url) = hook.slack_webhook {
        let (icon, status_word) = if result == 0 {
            (":white_check_mark:", "succeeded")
        } else {
            (":x:", "failed")
        };
        let msg = format!(
            "{} Pipeline {} for `{}` — exit_code={} duration={}s",
            icon, status_word, hook.guid, result, duration
        );
        send_slack(url, &msg, &state.logger).await;
    }

    // Update last run info
    {
        let mut last_runs = state.last_runs.lock().await;
        last_runs.insert(
            hook.guid.clone(),
            LastRunInfo {
                delivery_id: info.delivery_id.clone(),
                started_at,
                finished_at,
                exit_code: result,
                duration_seconds: duration,
            },
        );
    }

    // Release lock and decrement active count
    {
        let mut locks = state.run_locks.lock().await;
        locks.remove(&hook.guid);
    }
    state.active_count.fetch_sub(1, Ordering::Relaxed);
}

// --- TLS loading (no mTLS) ---

fn load_tls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<rustls::ServerConfig, Box<dyn std::error::Error>> {
    let cert_file = fs::File::open(cert_path)?;
    let mut cert_reader = BufReader::new(cert_file);
    let server_certs: Vec<_> = certs(&mut cert_reader)?
        .into_iter()
        .map(rustls::Certificate)
        .collect();

    let key_file = fs::File::open(key_path)?;
    let mut key_reader = BufReader::new(key_file);

    let keys = pkcs8_private_keys(&mut key_reader)?;
    let key = if !keys.is_empty() {
        rustls::PrivateKey(keys[0].clone())
    } else {
        let mut key_reader = BufReader::new(fs::File::open(key_path)?);
        let rsa_keys = rsa_private_keys(&mut key_reader)?;
        if rsa_keys.is_empty() {
            return Err("No valid private key found".into());
        }
        rustls::PrivateKey(rsa_keys[0].clone())
    };

    let config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(server_certs, key)?;

    Ok(config)
}

// --- Hook loading ---

fn load_hooks(
    hooks_dir: &str,
    default_timeout: u64,
    logger_placeholder: &str,
) -> HashMap<String, HookConfig> {
    let mut hooks = HashMap::new();

    let entries = match fs::read_dir(hooks_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read hooks dir {}: {}", hooks_dir, e);
            return hooks;
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

        // Extract GUID: everything after last '-' before '.env'
        let stem = filename.strip_suffix(".env").unwrap();
        let guid = match stem.rsplit_once('-') {
            Some((_, guid)) => guid.to_string(),
            None => {
                eprintln!(
                    "{} Skipping hook file {} — no GUID in filename",
                    logger_placeholder, filename
                );
                continue;
            }
        };

        let env_vars = load_config(path.to_str().unwrap());

        let provider = match env_vars.get("HOOK_PROVIDER") {
            Some(p) => p.clone(),
            None => {
                eprintln!("Skipping hook {} — missing HOOK_PROVIDER", filename);
                continue;
            }
        };

        let secret = match env_vars.get("HOOK_SECRET") {
            Some(s) => s.clone(),
            None => {
                eprintln!("Skipping hook {} — missing HOOK_SECRET", filename);
                continue;
            }
        };

        let script = match env_vars.get("HOOK_SCRIPT") {
            Some(s) => s.clone(),
            None => {
                eprintln!("Skipping hook {} — missing HOOK_SCRIPT", filename);
                continue;
            }
        };

        // Warn if script doesn't exist
        if !std::path::Path::new(&script).exists() {
            eprintln!("Warning: hook {} script not found: {}", guid, script);
        }

        let timeout = env_vars
            .get("HOOK_TIMEOUT")
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_timeout);

        let dedupe_window = env_vars
            .get("HOOK_DEDUPE_WINDOW")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let slack_webhook = env_vars.get("HOOK_SLACK_WEBHOOK").cloned()
            .filter(|s| !s.is_empty());

        let hook = HookConfig {
            guid: guid.clone(),
            provider,
            secret,
            script,
            timeout,
            dedupe_window,
            slack_webhook,
            env_vars,
        };

        if hook.slack_webhook.is_none() {
            println!("  HOOK_SLACK_WEBHOOK not defined in env var; slack notification disabled for guid={}", guid);
        }
        println!("  Loaded hook: {} (guid={}, provider={})", filename, guid, hook.provider);
        hooks.insert(guid, hook);
    }

    hooks
}

// --- main ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("shrmpl-cicd-srv version {}", VERSION);
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }

    let config = load_config(&args[1]);

    // Extract server config
    let tls_mode = config
        .get("CICD_TLS_MODE")
        .unwrap_or(&"plain".to_string())
        .clone();
    let listen_addr = config
        .get("CICD_LISTEN_ADDR")
        .expect("CICD_LISTEN_ADDR required")
        .clone();
    let listen_port: u16 = config
        .get("CICD_LISTEN_PORT")
        .expect("CICD_LISTEN_PORT required")
        .parse()
        .expect("CICD_LISTEN_PORT must be a valid port");
    let tls_cert = config
        .get("CICD_TLS_CERT")
        .unwrap_or(&String::new())
        .clone();
    let tls_key = config
        .get("CICD_TLS_KEY")
        .unwrap_or(&String::new())
        .clone();
    let hooks_dir = config
        .get("CICD_HOOKS_DIR")
        .expect("CICD_HOOKS_DIR required")
        .clone();
    let max_concurrent: usize = config
        .get("CICD_MAX_CONCURRENT")
        .expect("CICD_MAX_CONCURRENT required")
        .parse()
        .expect("CICD_MAX_CONCURRENT must be a number");
    let default_timeout: u64 = config
        .get("CICD_DEFAULT_TIMEOUT")
        .expect("CICD_DEFAULT_TIMEOUT required")
        .parse()
        .expect("CICD_DEFAULT_TIMEOUT must be a number");

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
        "shrmpl-cicd".to_string(),
        shrmpl::shrmpl_log_client::LogLevel::from_str(&slog_level),
        slog_console,
        slog_send_actv,
        slog_send_log,
    );

    // Load hooks
    println!("Loading hooks from {}...", hooks_dir);
    let hooks = load_hooks(&hooks_dir, default_timeout, "CICDHOOK");

    if hooks.is_empty() {
        eprintln!("Warning: no hooks loaded from {}", hooks_dir);
    }

    let server_config = ServerConfig {
        tls_mode: tls_mode.clone(),
        listen_addr: listen_addr.clone(),
        listen_port,
        tls_cert,
        tls_key,
        hooks_dir,
        max_concurrent,
        default_timeout,
    };

    let state = Arc::new(AppState {
        hooks,
        run_locks: Mutex::new(HashSet::new()),
        dedupe_buffers: Mutex::new(HashMap::new()),
        active_count: AtomicUsize::new(0),
        config: server_config,
        start_time: Instant::now(),
        last_runs: Mutex::new(HashMap::new()),
        logger: logger.clone(),
    });

    let addr: SocketAddr = format!("{}:{}", listen_addr, listen_port).parse()?;

    let start_msg = format!(
        "shrmpl-cicd-srv version {} listening on {} (tls_mode={}, hooks={})",
        VERSION,
        addr,
        tls_mode,
        state.hooks.len()
    );
    println!("{}", start_msg);
    logger.info("CICDSTART", &start_msg).await;

    // Log each loaded hook
    for (guid, hook) in &state.hooks {
        logger
            .info(
                "CICDHOOK",
                &format!(
                    "Hook loaded: guid={} provider={} script={}",
                    guid, hook.provider, hook.script
                ),
            )
            .await;
    }

    let listener = TcpListener::bind(&addr).await?;

    if tls_mode == "tls" {
        if state.config.tls_cert.is_empty() || state.config.tls_key.is_empty() {
            eprintln!("CICD_TLS_CERT and CICD_TLS_KEY required when CICD_TLS_MODE=tls");
            std::process::exit(1);
        }

        let tls_config = load_tls_config(&state.config.tls_cert, &state.config.tls_key)?;
        let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

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
                            match tls_acceptor.accept(stream).await {
                                Ok(tls_stream) => yield Ok::<_, hyper::Error>(tls_stream),
                                Err(e) => {
                                    eprintln!("TLS handshake failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to accept connection: {}", e);
                        }
                    }
                }
            },
        ))
        .serve(make_svc);

        logger.info("CICDSTART", "TLS server started").await;

        if let Err(e) = server.await {
            logger
                .error("CICDSHUT", &format!("Server error: {}", e))
                .await;
        }
    } else {
        // Plain HTTP mode
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

        logger.info("CICDSTART", "Plain HTTP server started").await;

        if let Err(e) = server.await {
            logger
                .error("CICDSHUT", &format!("Server error: {}", e))
                .await;
        }
    }

    Ok(())
}
