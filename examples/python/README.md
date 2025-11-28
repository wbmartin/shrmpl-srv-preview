# Shrmpl Python Examples

This directory contains Python client library and examples for Shrmpl services.

## Files

- `shrmpl.py` - Complete client library for KV, Log, and Vault services
- `example.py` - Working example demonstrating all three services

## Usage

### KV Server
```python
from shrmpl import ShrmplKV

kv = ShrmplKV("127.0.0.1", 7171)
success, error = kv.connect()
if success:
    # Set with TTL
    kv.set("key", "value", "5s")
    
    # Get value
    value, error = kv.get("key")
    
    # Increment counter
    count, error = kv.incr("counter", "1min")
```

### Log Server
```python
from shrmpl import ShrmplLog

log = ShrmplLog("127.0.0.1", 7379)
success, error = log.connect()
if success:
    # Send log message
    log.send("INFO", "my-host", "T001", "Application started")
```

### Vault Server
```python
from shrmpl import ShrmplVault

vault = ShrmplVault(
    server_url="https://vault.example.com:7474",
    cert_path="/path/to/client.crt",
    key_path="/path/to/client.key", 
    secret="my_secret"
)
success, error = vault.connect()
if success:
    # Get configuration
    content, error = vault.get_config("config-file-name")
```

## Running the Example

1. Start required Shrmpl servers:
   ```bash
   shrmpl-kv-srv etc/shrmpl-kv-srv-loc.env
   shrmpl-log-srv etc/shrmpl-log-srv-loc.env
   shrmpl-vault-srv etc/shrmpl-vault-srv-loc.env
   ```

2. Run the example:
   ```bash
   python3 example.py
   ```

## Error Handling

All library methods return `(result, error)` tuples:
- `error` is `None` on success
- `result` contains the successful response
- Check `error` first before using `result`

## Features

- **Persistent connections** - Connect once, reuse for multiple operations
- **Tuple returns** - `(result, error)` pattern for flexible error handling
- **Protocol compliance** - Implements exact wire protocols from tech specs
- **Drop-in ready** - Just copy `shrmpl.py` to your project