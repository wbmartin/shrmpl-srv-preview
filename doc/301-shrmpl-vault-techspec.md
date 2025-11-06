# shrmpl-vault Technical Specification

## Overview
shrmpl-vault is a secure configuration and secret management service that provides HTTPS with mutual TLS authentication for retrieving configuration files. It follows the shrmpl philosophy of simplicity and reliability.

## Architecture

### Server Component (`shrmpl_vault_srv`)
- HTTPS server with mutual TLS authentication using rustls + hyper
- File-based configuration storage in a single directory
- Rate limiting per client certificate
- Integration with shrmpl-log for logging
- Simple GET-only API for file retrieval

### Client Component (`shrmpl_vault_cli`)
- Command-line client for retrieving configurations
- Mutual TLS certificate presentation
- Secret key authentication via query string
- Simple single-request interface

## Configuration

### Server Configuration (.env format)
```bash
# Network binding
BIND_ADDR=0.0.0.0:7474

# Logging configuration
SLOG_DEST=127.0.0.1:7379
SERVER_NAME=shrmpl-vault-loc
SEND_LOG=true
LOG_LEVEL=DEBUG
LOG_CONSOLE=true
SEND_ACTV=false

# TLS configuration
TLS_CERTIFICATE_PRIVKEY_PATH=/path/to/privkey.pem
TLS_CERTIFICATE_FULLCHAIN_PATH=/path/to/fullchain.pem

# File storage
CONFIG_DIR=/path/to/config/files

# Security
ALLOWED_SECRETS=secret1,secret2,secret3
RATE_LIMIT_REQUESTS_PER_MINUTE=60
```

### Client Configuration (.env format)
```bash
# Server connection
VAULT_SERVER=https://vault.example.com:7474

# TLS certificates
CLIENT_CERT_PATH=/path/to/client.crt
CLIENT_KEY_PATH=/path/to/client.key

# Authentication
SECRET_KEY=secret1

# Request target
FILENAME=dev_simple-example_app-server-config-json_08ff3053-b7ba-4f8a-a0d5-b4107c3fc319
```

## API Specification

### Endpoint
```
GET /{filename}?secret={secret_key}
```

### Response Codes
- `200 OK`: File retrieved successfully
- `404 Not Found`: File does not exist
- `401 Unauthorized`: Invalid client certificate or secret key
- `429 Too Many Requests`: Rate limit exceeded
- `500 Internal Server Error`: Server error

### Response Headers
- `Content-Type: text/plain`
- `Content-Length`: File size in bytes

## File Naming Convention
Files follow the pattern: `[environment]-[appname]-[friendlyname]-[guid]`
Example: `dev_simple-example_app-server-config-json_08ff3053-b7ba-4f8a-a0d5-b4107c3fc319`

The server does not validate this format but it's recommended for organization.

## Security Features

### Mutual TLS Authentication
- Server presents certificate chain
- Client must present valid certificate
- Certificate expiration checking on both sides
- rustls for secure TLS implementation
- TLS 1.3 minimum version requirement
- Server logs certificate expiration days on startup

### Secret Key Authentication
- Additional authentication via query string parameter
- Server validates against `ALLOWED_SECRETS` list (exact string matches only)
- Allows revocation without certificate changes

### Rate Limiting
- Per-secret-key rate limiting (simpler than certificate fingerprint)
- Simple HashMap tracking request counts with 60-second windows
- Configurable requests per minute via `RATE_LIMIT_REQUESTS_PER_MINUTE`
- Prevents abuse and brute force attacks

## Certificate Generation

### Server Certificate Generation
```bash
# Generate server private key (PEM format by default)
openssl genrsa -out shrmpl_vault_server_privkey.pem 2048

# Generate server certificate signing request
openssl req -new -key shrmpl_vault_server_privkey.pem -out server.csr

# Generate server certificate (self-signed for development, PEM format by default)
openssl x509 -req -days 365 -in server.csr -signkey shrmpl_vault_server_privkey.pem -out shrmpl_vault_server_fullchain.pem
```

### Client Certificate Generation
```bash
# Generate client private key (PEM format by default)
openssl genrsa -out shrmpl_vault_client_privkey.pem 2048

# Generate client certificate signing request
openssl req -new -key shrmpl_vault_client_privkey.pem -out client.csr

# Generate client certificate (signed by server CA or self-signed, PEM format by default)
openssl x509 -req -days 365 -in client.csr -signkey shrmpl_vault_server_privkey.pem -out shrmpl_vault_client_cert.pem

# Convert to PKCS8 format for rustls compatibility
openssl pkcs8 -topk8 -inform PEM -outform PEM -in shrmpl_vault_client_privkey.pem -out shrmpl_vault_client_privkey_pkcs8.pem -nocrypt
```

## Implementation Details

### Dependencies
- `tokio` for async runtime
- `rustls` for TLS implementation
- `hyper` for HTTP server
- `tracing` for logging integration
- Existing `config.rs` for .env configuration parsing
- Simple in-memory rate limiting using HashMap and timestamps

### File Operations
- Files read directly from disk on each request (no caching)
- Concurrent file access handled by OS
- File size guideline: 3KB maximum (not enforced)
- Supported file types: plain text, JSON, YAML

### Logging Integration
All requests logged with:
- Client IP address
- Requested filename
- Authentication result
- Response status code
- Timestamp

Log levels:
- `DEBUG`: Detailed request information
- `INFO`: Successful file retrievals
- `WARN`: Authentication failures
- `ERROR`: Server errors

### Health Check
Special file `healthcheck` with content `ok` for health monitoring
Accessible at `/healthcheck` endpoint
Requires valid client certificate (secret key still needed)

## Error Handling

### Certificate Errors
- Expired certificates: 401 Unauthorized
- Invalid certificates: 401 Unauthorized
- Missing certificates: 401 Unauthorized

### Authentication Errors
- Invalid secret key: 401 Unauthorized
- Missing secret key: 401 Unauthorized

### File Errors
- File not found: 404 Not Found
- File read error: 500 Internal Server Error

### Rate Limiting
- Rate limit exceeded: 429 Too Many Requests
- Retry-After header included



## Testing Strategy

### Unit Tests
- Configuration parsing
- Secret key validation
- Rate limiting logic
- File operations

### Integration Tests
- Mutual TLS handshake
- End-to-end file retrieval
- Error handling scenarios
- Rate limiting enforcement

### Security Tests
- Certificate validation
- Secret key bypass attempts
- Rate limit circumvention
- File path traversal attempts

## Deployment Considerations

### File Permissions
- Configuration directory: readable by server process only
- TLS certificates: readable by server process only
- Log files: appropriate permissions for rotation

### Process Management
- Run as non-root user
- Signal handling for graceful shutdown
- PID file management

### Monitoring
- Certificate expiration: server logs days until expiration on startup
- Rate limit hit monitoring
- Error rate tracking
- Response time metrics
