# Shrmpl Vault Mutual TLS (mTLS)

The `shrmpl-vault` server uses Mutual TLS (mTLS) to authenticate clients. This provides a strong security layer by ensuring that only clients with a valid certificate signed by a trusted Certificate Authority (CA) can access the vault.

## TLS vs. mTLS

In a standard TLS connection, only the client verifies the server's identity. The server presents its certificate, and the client checks if it trusts that certificate.

![Standard TLS](img/shrmpl-kv.png)

With mTLS, the server also verifies the client's identity. The process is as follows:

1.  The client connects to the server.
2.  The server presents its TLS certificate.
3.  The client verifies the server's certificate.
4.  The client presents its own certificate to the server.
5.  The server verifies the client's certificate against a pre-configured list of trusted CAs.
6.  If both verifications are successful, the connection is established.

![mTLS](img/shrmpl-log.png)

This means we need two sets of certificates:

1.  **Server TLS Certificate**: For the server to prove its identity to clients. These are configured via `TLS_CERTIFICATE_PRIVKEY_PATH` and `TLS_CERTIFICATE_FULLCHAIN_PATH` in the environment configuration.
2.  **Client mTLS Certificates**: For clients to prove their identity to the server. This involves:
    *   A **Certificate Authority (CA)** that the server trusts. The server is configured with this CA certificate.
    *   **Client certificates** (and private keys) signed by that CA. Clients must use these to connect.

## Certificate Generation

To facilitate the setup of mTLS, a script is provided to generate the necessary CA and client certificates.

### Usage

To generate the certificates, run the following command from the project root:

```bash
./bin/302-generate-vault-mtls-certs.sh
```

This script will create the following files in the `etc/crt/` directory:

*   `shrmpl_vault_mtls_ca.pem`: The Certificate Authority certificate. This is what the server uses to verify clients.
*   `shrmpl_vault_mtls_client_cert.pem`: The client certificate.
*   `shrmpl_vault_mtls_client_privkey.pem`: The client's private key.

The script is idempotent and will not overwrite existing certificates.

## Server Configuration

The `shrmpl-vault` server needs to be pointed to the CA certificate to use for client verification. This is done via the `MTLS_CLIENT_CA_CERT_PATH` variable in the configuration file (e.g., `etc/shrmpl-vault-srv-loc.env`):

```
# mTLS configuration
MTLS_CLIENT_CA_CERT_PATH=/path/to/your/project/etc/crt/shrmpl_vault_mtls_ca.pem
```

When the server starts, it will load this CA and will only accept connections from clients that present a certificate signed by it.
