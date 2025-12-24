#!/bin/bash

# This script generates a Certificate Authority (CA) and a client certificate/key
# for use with shrmpl-vault mTLS.

set -e
set -o pipefail

# Check for openssl
if ! command -v openssl &> /dev/null
then
    echo "openssl could not be found. Please install it."
    exit 1
fi

# Define paths
BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
CERT_DIR="$BASE_DIR/etc/crt"
CA_KEY="$CERT_DIR/shrmpl_vault_mtls_ca_privkey.pem"
CA_CERT="$CERT_DIR/shrmpl_vault_mtls_ca.pem"
CLIENT_KEY="$CERT_DIR/shrmpl_vault_mtls_client_privkey.pem"
CLIENT_CSR="$CERT_DIR/shrmpl_vault_mtls_client.csr"
CLIENT_CERT="$CERT_DIR/shrmpl_vault_mtls_client_cert.pem"
SERIAL_FILE="$CERT_DIR/shrmpl_vault_mtls_ca.srl"

# Create cert directory if it doesn't exist
mkdir -p "$CERT_DIR"

# --- Certificate Authority ---

if [ -f "$CA_CERT" ]; then
    echo "CA certificate already exists, skipping generation: $CA_CERT"
else
    echo "Generating CA private key..."
    openssl genrsa -out "$CA_KEY" 2048

    echo "Generating CA root certificate..."
    openssl req -x509 -new -nodes -key "$CA_KEY" \
        -sha256 -days 3650 \
        -out "$CA_CERT" \
        -subj "/C=US/ST=California/L=San Francisco/O=Shrmpl/CN=ShrmplVaultMTLS_CA"
    echo "CA certificate created: $CA_CERT"
fi


# --- Client Certificate ---

if [ -f "$CLIENT_CERT" ]; then
    echo "Client certificate already exists, skipping generation: $CLIENT_CERT"
else
    echo "Generating client private key..."
    openssl genrsa -out "$CLIENT_KEY" 2048

    echo "Generating client certificate signing request (CSR)..."
    openssl req -new -key "$CLIENT_KEY" -out "$CLIENT_CSR" \
        -subj "/C=US/ST=California/L=San Francisco/O=Shrmpl/CN=shrmpl-vault-client"

    echo "Signing client certificate with CA..."
    # The serial file is needed to track certificates issued by the CA
    if [ ! -f "$SERIAL_FILE" ]; then
        echo "01" > "$SERIAL_FILE"
    fi
    
    openssl x509 -req -in "$CLIENT_CSR" \
        -CA "$CA_CERT" -CAkey "$CA_KEY" \
        -CAserial "$SERIAL_FILE" -out "$CLIENT_CERT" \
        -days 365 -sha256

    # Clean up the CSR
    rm "$CLIENT_CSR"

    echo "Client certificate created: $CLIENT_CERT"
    echo "Client private key created: $CLIENT_KEY"
fi

echo "mTLS certificate generation complete."
