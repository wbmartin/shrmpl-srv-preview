# Shrmpl Go Examples

This directory contains Go client library and examples for Shrmpl services.

## Files

- `shrmpl/` - Client library package
  - `shrmpl.go` - Complete client library for KV, Log, and Vault services
  - `go.mod` - Go module definition
- `main.go` - Working example demonstrating all three services
- `go.mod` - Go module for the example

## Usage

### KV Server
```go
package main

import (
    "fmt"
    "shrmpl"
)

func main() {
    kv := shrmpl.NewKVClient("127.0.0.1", 7171)
    success, err := kv.Connect()
    if success {
        // Set with TTL
        kv.Set("key", "value", "5s")
        
        // Get value
        value, err := kv.Get("key")
        
        // Increment counter
        count, err := kv.Incr("counter", "1min")
    }
}
```

### Log Server
```go
package main

import (
    "fmt"
    "shrmpl"
)

func main() {
    log := shrmpl.NewLogClient("127.0.0.1", 7379)
    success, err := log.Connect()
    if success {
        // Send log message
        err := log.Send("INFO", "my-host", "T001", "Application started")
    }
}
```

### Vault Server
```go
package main

import (
    "fmt"
    "shrmpl"
)

func main() {
    vault := shrmpl.NewVaultClient(
        "https://vault.example.com:7474",
        "/path/to/client.crt",
        "/path/to/client.key",
        "my_secret",
    )
    success, err := vault.Connect()
    if success {
        // Get configuration
        content, err := vault.GetConfig("config-file-name")
    }
}
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
   cd examples/go
   go run main.go
   ```

## Error Handling

All library methods return `(result, error)` tuples:
- `error` is `nil` on success
- `result` contains the successful response
- Check `err != nil` before using the result

## Features

- **Persistent connections** - Connect once, reuse for multiple operations
- **Tuple returns** - `(result, error)` pattern for flexible error handling
- **Protocol compliance** - Implements exact wire protocols from tech specs
- **Drop-in ready** - Just copy the `shrmpl/` package to your project

## Module Structure

The library is structured as a Go module:
- `shrmpl/` package contains all client code
- Uses Go modules for dependency management
- Import as `"shrmpl"` in your code