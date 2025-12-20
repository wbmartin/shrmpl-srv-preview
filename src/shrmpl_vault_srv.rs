const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::collections::HashMap;
use std::fs;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};
use x509_parser::prelude::*;

use shrmpl::config::load_config;
use shrmpl::shrmpl_log_client::Logger;

#[derive(Clone)]
struct RateLimiter {
    requests: Arc<std::sync::Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests_per_minute: u32,
}

impl RateLimiter {
    fn new(max_requests_per_minute: u32) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(HashMap::new())),
            max_requests_per_minute,
        }
    }

    fn check_rate_limit(&self, secret_key: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);

        let entry = requests.entry(secret_key.to_string()).or_insert_with(Vec::new);
        entry.retain(|&timestamp| timestamp > one_minute_ago);

        if entry.len() < self.max_requests_per_minute as usize {
            entry.push(now);
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
struct VaultState {
    config_dir: String,
    allowed_secrets: Vec<String>,
    rate_limiter: RateLimiter,
    logger: Logger,
}

async fn handle_request(req: Request<Body>, state: VaultState) -> Result<Response<Body>, hyper::Error> {
    let method = req.method();
    let uri = req.uri();
    let client_ip = get_client_ip(&req);

    if method != &Method::GET {
        let msg = format!("{} {} - Method not allowed: {}", client_ip, method, uri);
        warn!("{}", msg);
        state.logger.warn("HTTPERROR", &msg).await;
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Body::from("Method not allowed"))
            .unwrap());
    }

    let path = uri.path();
    let query_params = parse_query_params(uri.query());

    // Check for secret key in query params
    let secret_key = match query_params.get("secret") {
        Some(key) => key,
        None => {
            let msg = format!("{} {} - Missing secret key", client_ip, uri);
            warn!("{}", msg);
            state.logger.warn("AUTHFAIL", &msg).await;
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from("Missing secret key"))
                .unwrap());
        }
    };

    // Validate secret key
    if !state.allowed_secrets.contains(secret_key) {
        let msg = format!("{} {} - Invalid secret key: {}", client_ip, uri, secret_key);
        warn!("{}", msg);
        state.logger.warn("AUTH", &msg).await;
        return Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("Invalid secret key"))
            .unwrap());
    }

    // Check rate limit
    if !state.rate_limiter.check_rate_limit(secret_key) {
        let msg = format!("{} {} - Rate limit exceeded for secret: {}", client_ip, uri, secret_key);
        warn!("{}", msg);
        state.logger.warn("RATELIMIT", &msg).await;
        return Ok(Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Retry-After", "60")
            .body(Body::from("Rate limit exceeded"))
            .unwrap());
    }



    // Extract filename from path (remove leading slash)
    let filename = match path.strip_prefix("/") {
        Some(name) => name,
        None => {
            let msg = format!("{} {} - Invalid path format", client_ip, uri);
            warn!("{}", msg);
            state.logger.warn("HTTPERROR", &msg).await;
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("Invalid path"))
                .unwrap());
        }
    };

    // Construct full file path
    let file_path = format!("{}/{}", state.config_dir, filename);

    // Read and return file
    match fs::read_to_string(&file_path) {
        Ok(content) => {
            let msg = format!("{} {} - Successfully retrieved file: {}", client_ip, uri, filename);
            info!("{}", msg);
            state.logger.activity("VAULTACCESS", &msg).await;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .header("Content-Length", content.len().to_string())
                .body(Body::from(content))
                .unwrap())
        }
        Err(_) => {
            let msg = format!("{} {} - File not found: {}", client_ip, uri, filename);
            warn!("{}", msg);
            state.logger.warn("FILENOTFND", &msg).await;
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("File not found"))
                .unwrap())
        }
    }
}

fn get_client_ip(req: &Request<Body>) -> String {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            req.headers()
                .get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_query_params(query: Option<&str>) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query_str) = query {
        for pair in query_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                params.insert(key.to_string(), value.to_string());
            }
        }
    }
    params
}

fn check_certificate_expiration(cert_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cert_pem = fs::read(cert_path)?;
    
    // Parse PEM to extract DER certificate
    let mut cert_reader = BufReader::new(&cert_pem[..]);
    let certs = rustls_pemfile::certs(&mut cert_reader)?;
    
    if certs.is_empty() {
        return Err("No certificates found in PEM file".into());
    }
    
    // Parse the first certificate as DER
    match parse_x509_certificate(&certs[0]) {
        Ok((_, cert)) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            let not_after = cert.validity().not_after.timestamp();
            let days_until_expiry = (not_after - now) / 86400;

            if days_until_expiry < 0 {
                error!("Certificate has expired!");
            } else if days_until_expiry < 30 {
                warn!("Certificate expires in {} days", days_until_expiry);
            } else {
                info!("Certificate expires in {} days", days_until_expiry);
            }

            Ok(())
        }
        Err(e) => {
            error!("Failed to parse certificate: {}", e);
            if !certs.is_empty() {
                let first_bytes = &certs[0][..std::cmp::min(20, certs[0].len())];
                error!("First 20 bytes of certificate DER: {:?}", first_bytes);
            }
            Err(Box::new(e))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("shrmpl-vault-srv version {}", VERSION);
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }

    let config = load_config(&args[1]);

    // Extract configuration values
    let bind_addr = config.get("BIND_ADDR").unwrap_or(&"0.0.0.0:7474".to_string()).clone();
    let log_level = config.get("LOG_LEVEL").unwrap_or(&"DEBUG".to_string()).clone();
    
    let cert_privkey_path = config.get("TLS_CERTIFICATE_PRIVKEY_PATH")
        .expect("TLS_CERTIFICATE_PRIVKEY_PATH required");
    let cert_fullchain_path = config.get("TLS_CERTIFICATE_FULLCHAIN_PATH")
        .expect("TLS_CERTIFICATE_FULLCHAIN_PATH required");
    
    let config_dir = config.get("CONFIG_DIR")
        .expect("CONFIG_DIR required");
    let allowed_secrets_str = config.get("ALLOWED_SECRETS")
        .expect("ALLOWED_SECRETS required");
    let default_rate_limit = "60".to_string();
    let rate_limit_str = config.get("RATE_LIMIT_REQUESTS_PER_MINUTE")
        .unwrap_or(&default_rate_limit);

    // Logging configuration
    let slog_dest = config.get("SLOG_DEST").unwrap_or(&"".to_string()).clone();
    let server_name = config.get("SERVER_NAME").unwrap_or(&"shrmpl-vault".to_string()).clone();
    let send_log = config.get("SEND_LOG").map(|s| s.parse().unwrap_or(true)).unwrap_or(true);
    let log_console = config.get("LOG_CONSOLE").map(|s| s.parse().unwrap_or(true)).unwrap_or(true);
    let send_actv = config.get("SEND_ACTV").map(|s| s.parse().unwrap_or(false)).unwrap_or(false);

    // Parse allowed secrets
    let allowed_secrets: Vec<String> = allowed_secrets_str
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Parse rate limit
    let rate_limit: u32 = rate_limit_str.parse().unwrap_or(60);

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(match log_level.as_str() {
            "DEBUG" => tracing::Level::DEBUG,
            "INFO" => tracing::Level::INFO,
            "WARN" => tracing::Level::WARN,
            "ERROR" => tracing::Level::ERROR,
            _ => tracing::Level::INFO,
        })
        .init();

    // Check certificate expiration
    let cert_check_msg = "Checking certificate expiration...";
    info!("{}", cert_check_msg);
    if let Err(e) = check_certificate_expiration(cert_fullchain_path) {
        let msg = format!("Failed to check certificate expiration: {}", e);
        error!("{}", msg);
    }

    // Initialize rate limiter
    let rate_limiter = RateLimiter::new(rate_limit);

    // Initialize logger
    let logger = Logger::new(
        slog_dest,
        server_name,
        shrmpl::shrmpl_log_client::LogLevel::from_str(&log_level),
        log_console,
        send_actv,
        send_log,
    );

    // Create vault state
    let state = VaultState {
        config_dir: config_dir.clone(),
        allowed_secrets,
        rate_limiter,
        logger,
    };

    // Log certificate check
    state.logger.info("CERTCHECK", "Checking certificate expiration...").await;
    if let Err(e) = check_certificate_expiration(cert_fullchain_path) {
        let msg = format!("Failed to check certificate expiration: {}", e);
        error!("{}", msg);
        state.logger.error("CERTCHECK", &msg).await;
    }

    // Load TLS certificates
    let tls_config = match load_server_config(cert_privkey_path, cert_fullchain_path) {
        Ok(config) => config,
        Err(e) => {
            let msg = format!("Failed to load TLS configuration: {}", e);
            error!("{}", msg);
            return Err(e);
        }
    };

    // Create TLS acceptor
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Parse bind address
    let addr: SocketAddr = bind_addr.parse()?;

    // Create TCP listener
    let listener = TcpListener::bind(&addr).await?;
    let start_msg = format!("shrmpl-vault-srv version {} listening on {}", VERSION, addr);
    info!("{}", start_msg);
    state.logger.info("VAULTLISTEN", &start_msg).await;

    // Clone state for logging after server creation
    let state_for_logging = state.clone();
    
    // Create service
    let make_svc = make_service_fn(move |_conn| {
        let state = state.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(req, state.clone())
            }))
        }
    });

    // Create server
    let server = Server::builder(hyper::server::accept::from_stream(
        async_stream::stream! {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        match tls_acceptor.accept(stream).await {
                            Ok(tls_stream) => yield Ok::<_, hyper::Error>(tls_stream),
                            Err(e) => {
                                let msg = format!("TLS handshake failed: {}", e);
                                error!("{}", msg);
                                // Note: Can't log to SLOG here as we're outside the request handler
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Failed to accept connection: {}", e);
                        error!("{}", msg);
                        // Note: Can't log to SLOG here as we're outside the request handler
                    }
                }
            }
        }
    ))
    .serve(make_svc);

    let success_msg = "shrmpl-vault server started successfully";
    info!("{}", success_msg);
    state_for_logging.logger.info("SRVU", success_msg).await;
    
    if let Err(e) = server.await {
        let msg = format!("Server error: {}", e);
        error!("{}", msg);
        state_for_logging.logger.error("SRVU", &msg).await;
    }

    Ok(())
}

fn load_server_config(
    privkey_path: &str,
    fullchain_path: &str,
) -> Result<ServerConfig, Box<dyn std::error::Error>> {
    // Load and parse certificate
    let cert_file = fs::File::open(fullchain_path)?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = certs(&mut cert_reader)?
        .into_iter()
        .map(rustls::Certificate)
        .collect();

    // Load and parse private key
    let key_file = fs::File::open(privkey_path)?;
    let mut key_reader = BufReader::new(key_file);
    
    // Try PKCS8 first, then RSA
    let keys = pkcs8_private_keys(&mut key_reader)?;
    let key = if !keys.is_empty() {
        rustls::PrivateKey(keys[0].clone())
    } else {
        // Reset reader and try RSA keys
        let mut key_reader = BufReader::new(fs::File::open(privkey_path)?);
        let rsa_keys = rsa_private_keys(&mut key_reader)?;
        if rsa_keys.is_empty() {
            return Err("No valid private key found".into());
        }
        rustls::PrivateKey(rsa_keys[0].clone())
    };

    let config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(config)
}