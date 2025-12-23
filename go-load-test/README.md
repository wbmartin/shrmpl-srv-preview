# Go Load Test for shrmpl-kv

This is a Go-based load testing tool for the shrmpl-kv key-value store, designed to mirror the functionality of the Rust load test while using the advanced Go client with connection pooling and reconnection.

## Features

- **Connection Modes**: Default shared connection (simulates Golang client queuing) or individual connections per user
- **Test Modes**: Simple batch GET operations or comprehensive testing (SET/GET/INCR with verification)
- **Performance Metrics**: Response time bucketing, success rates, total test duration
- **Error Handling**: Detailed error reporting and categorization

## Usage

```bash
# Build the load test
go build

# Run basic batch GET test with shared connection (default)
./go-load-test etc/shrmpl-kv-srv-loc.env

# Run comprehensive test with shared connection
./go-load-test --full etc/shrmpl-kv-srv-loc.env

# Run with individual connections per user
./go-load-test --multi etc/shrmpl-kv-srv-loc.env

# Run comprehensive test with individual connections
./go-load-test --full --multi etc/shrmpl-kv-srv-loc.env
```

## Options

- `--multi`: Use individual connections per user instead of shared connection (default: shared)
- `--full`: Run comprehensive test with SET/GET/INCR verification instead of just batch GET

## Output Format

Matches the Rust load test output for easy comparison:

```
Load Test Results:
Total Operations: 5000
Successful: 5000 (100.0%)
Errors: 0 (0.0%)

Response Time Distribution (successful operations):
<10ms: 5000 (100.0%)
<50ms: 0 (0.0%)
...
Total Test Duration: 1.23s
```

## Architecture

- Uses the advanced shrmpl-kv Go client with automatic reconnection
- Implements connection pooling for shared connection mode
- Concurrent testing with goroutines
- Comprehensive error handling and cleanup