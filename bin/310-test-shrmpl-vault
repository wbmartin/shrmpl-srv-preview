#!/bin/bash

# This script tests the shrmpl-vault server with an mTLS client.

set -e
set -o pipefail

BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
VAULT_BINARY="dist/mac/shrmpl-vault-srv"
CONFIG_FILE="etc/shrmpl-vault-srv-loc.env"
CLIENT_CERT="etc/crt/shrmpl_vault_mtls_client_cert.pem"
CLIENT_KEY="etc/crt/shrmpl_vault_mtls_client_privkey.pem"
SERVER_CERT="etc/crt/shrmpl_vault_server_fullchain.pem"
TEST_CONFIG_FILE="etc/vault-configs/test-config"
EXPECTED_CONTENT=$(cat "$BASE_DIR/$TEST_CONFIG_FILE")

echo "--- Starting shrmpl-vault test ---"

# 1. Build the server
echo "Building server..."
"$BASE_DIR/bin/301-build-shrmpl-vault-release"

# 2. Stop any running servers
echo "Stopping any running vault servers..."
pkill -f shrmpl-vault-srv || true
sleep 1

# 3. Start the new server
echo "Starting shrmpl-vault-srv in the background..."
"$BASE_DIR/$VAULT_BINARY" "$BASE_DIR/$CONFIG_FILE" &
SERVER_PID=$!
echo "Server started with PID: $SERVER_PID"
sleep 2 # Give the server a moment to start

# 4. Test with curl
echo "Attempting to fetch config with mTLS client..."
RESPONSE=$(curl -s --insecure \
     --cert "$BASE_DIR/$CLIENT_CERT" \
     --key "$BASE_DIR/$CLIENT_KEY" \
     "https://localhost:7474/test-config?secret=test-secret-key")

# 5. Cleanup: Stop the server
echo "Stopping server with PID: $SERVER_PID"
kill $SERVER_PID
wait $SERVER_PID || true

# 6. Verify the response
echo "Verifying response..."
if [ "$RESPONSE" == "$EXPECTED_CONTENT" ]; then
    echo "SUCCESS: Received expected content from vault."
    echo "--- Test complete ---"
    exit 0
else
    echo "FAILURE: Did not receive expected content."
    echo "Expected: $EXPECTED_CONTENT"
    echo "Received: $RESPONSE"
    echo "--- Test failed ---"
    exit 1
fi
