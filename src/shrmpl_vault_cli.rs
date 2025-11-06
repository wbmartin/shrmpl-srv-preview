use std::fs;
use std::sync::Arc;
use std::io::BufReader;

use hyper::{Body, Client, Request, Uri};
use rustls::ClientConfig;
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tracing::{error, info};

use shrmpl::config::load_config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <config_file>", args[0]);
        std::process::exit(1);
    }

    let config = load_config(&args[1]);

    // Extract configuration values
    let vault_server = config.get("VAULT_SERVER")
        .expect("VAULT_SERVER required");
    let client_cert_path = config.get("CLIENT_CERT_PATH")
        .expect("CLIENT_CERT_PATH required");
    let client_key_path = config.get("CLIENT_KEY_PATH")
        .expect("CLIENT_KEY_PATH required");
    let secret_key = config.get("SECRET_KEY")
        .expect("SECRET_KEY required");
    let filename = config.get("FILENAME")
        .expect("FILENAME required");

    // Initialize logging
    tracing_subscriber::fmt::init();

    // Load client certificates
    let tls_config = load_client_config(client_cert_path, client_key_path)?;

    // Create HTTPS connector using hyper-rustls
    let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .build();

    // Create HTTP client
    let client = Client::builder().build::<_, Body>(https_connector);

    // Build request URL
    let url = format!("{}/{}?secret={}", vault_server.trim_end_matches('/'), filename, secret_key);
    let uri: Uri = url.parse()?;

    info!("Requesting file: {}", filename);

    // Create request
    let request = Request::builder()
        .method(hyper::Method::GET)
        .uri(uri)
        .header("User-Agent", "shrmpl-vault-cli/1.0")
        .body(Body::empty())?;

    // Send request
    let response = client.request(request).await?;

    let status = response.status();
    let headers = response.headers();

    // Handle response
    match status {
        hyper::StatusCode::OK => {
            let body_bytes = hyper::body::to_bytes(response.into_body()).await?;
            let content = String::from_utf8(body_bytes.to_vec())?;
            
            println!("{}", content);
            info!("Successfully retrieved file: {}", filename);
        }
        hyper::StatusCode::NOT_FOUND => {
            error!("File not found: {}", filename);
            eprintln!("Error: File not found");
            std::process::exit(1);
        }
        hyper::StatusCode::UNAUTHORIZED => {
            error!("Authentication failed for file: {}", filename);
            eprintln!("Error: Authentication failed");
            std::process::exit(1);
        }
        hyper::StatusCode::TOO_MANY_REQUESTS => {
            if let Some(retry_after) = headers.get("Retry-After") {
                if let Ok(retry_str) = retry_after.to_str() {
                    error!("Rate limit exceeded. Retry after: {} seconds", retry_str);
                    eprintln!("Error: Rate limit exceeded. Retry after: {} seconds", retry_str);
                } else {
                    error!("Rate limit exceeded");
                    eprintln!("Error: Rate limit exceeded");
                }
            } else {
                error!("Rate limit exceeded");
                eprintln!("Error: Rate limit exceeded");
            }
            std::process::exit(1);
        }
        _ => {
            error!("Server returned status: {}", status);
            eprintln!("Error: Server returned status: {}", status);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn load_client_config(
    cert_path: &str,
    key_path: &str,
) -> Result<ClientConfig, Box<dyn std::error::Error>> {
    // Load and parse certificate
    let cert_file = fs::File::open(cert_path)?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs = certs(&mut cert_reader)?
        .into_iter()
        .map(rustls::Certificate)
        .collect();

    // Load and parse private key
    let key_file = fs::File::open(key_path)?;
    let mut key_reader = BufReader::new(key_file);
    
    // Try PKCS8 first, then RSA
    let keys = pkcs8_private_keys(&mut key_reader)?;
    let key = if !keys.is_empty() {
        rustls::PrivateKey(keys[0].clone())
    } else {
        // Reset reader and try RSA keys
        let mut key_reader = BufReader::new(fs::File::open(key_path)?);
        let rsa_keys = rsa_private_keys(&mut key_reader)?;
        if rsa_keys.is_empty() {
            return Err("No valid private key found".into());
        }
        rustls::PrivateKey(rsa_keys[0].clone())
    };

    // For development, we'll use a config that doesn't verify server certificates
    // In production, you should use proper certificate verification
    let config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(DangerousNoVerification))
        .with_client_auth_cert(certs, key)?;

    Ok(config)
}

#[derive(Debug)]
struct DangerousNoVerification;

impl rustls::client::ServerCertVerifier for DangerousNoVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}