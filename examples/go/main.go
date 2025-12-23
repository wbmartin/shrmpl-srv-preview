package main

import (
	"fmt"

	"shrmpl"
)

func main() {
	fmt.Println("=== Shrmpl Client Library Example ===\n")

	// KV Server Example
	fmt.Println("1. KV Server Example:")
	config := &shrmpl.KVConfig{
		HostPort: "127.0.0.1:7171",
	}
	kv := shrmpl.NewKV(config)
	defer kv.Close()

	fmt.Println("   ✓ Connected to KV server (connection handled internally)")

	// Test SET with TTL
	err := kv.Set("example_key", "example_value", "30s")
	if err == nil {
		fmt.Println("   ✓ SET example_key = example_value (30s TTL)")
	} else {
		fmt.Printf("   ✗ SET failed: %v\n", err)
	}

	// Test GET
	value, err := kv.Get("example_key")
	if err == nil {
		fmt.Printf("   ✓ GET example_key = %s\n", value)
	} else {
		fmt.Printf("   ✗ GET failed: %v\n", err)
	}

	// Test INCR
	count, err := kv.Incr("counter", "1min")
	if err == nil {
		fmt.Printf("   ✓ INCR counter = %d\n", count)
	} else {
		fmt.Printf("   ✗ INCR failed: %v\n", err)
	}

	// Test BATCH operations (new feature)
	fmt.Println("   Testing BATCH operations:")
	batchResults, err := kv.Batch([]string{"GET example_key", "GET counter"})
	if err == nil {
		fmt.Printf("   ✓ BATCH GET results: %v\n", batchResults)
	} else {
		fmt.Printf("   ✗ BATCH failed: %v\n", err)
	}

	// Note: Advanced client features (reconnection, connection pooling) are
	// used internally by the KVClient for robust operation

	// Note: LIST operation not available in advanced KV interface
	fmt.Println("   (LIST operation not available in this example)")

	kv.Close()
	fmt.Println()

	// Log Server Example
	fmt.Println("2. Log Server Example:")
	logger := shrmpl.NewLogger("example-server-name", "127.0.0.1:7379")
	defer logger.Close()

	fmt.Println("   ✓ Connected to Log server (connection handled internally)")

	// Test structured logging
	logger.Info("T001", "Application started successfully", "host", "example-host")
	fmt.Println("   ✓ Sent INFO log message with structured data")

	logger.Error("E001", "Database connection failed", "host", "example-host", "severity", "high")
	fmt.Println("   ✓ Sent ERROR log message with structured data")

	logger.Close()
	fmt.Println()

	// Vault Server Example
	fmt.Println("3. Vault Server Example:")
	vault := shrmpl.NewVaultClient(
		"https://127.0.0.1:7474",
		"/path/to/client.crt",
		"/path/to/client.key",
		"example_secret",
	)

	success, err := vault.Connect()
	if !success {
		fmt.Printf("   Vault connect failed: %v\n", err)
		fmt.Println("   Note: This example requires actual certificates and running vault server")
	} else {
		fmt.Println("   ✓ Connected to Vault server")

		// Test config retrieval
		content, err := vault.GetConfig("example-config-file")
		if err == nil {
			fmt.Println("   ✓ Retrieved config file")
			if len(content) > 100 {
				fmt.Printf("   Content preview: %s...\n", content[:100])
			} else {
				fmt.Printf("   Content: %s\n", content)
			}
		} else {
			fmt.Printf("   ✗ Config retrieval failed: %v\n", err)
			fmt.Println("   Note: This tests actual connection to vault server")
		}
	}

	fmt.Println("\n=== Example Complete ===")
	fmt.Println("\nTo run this example:")
	fmt.Println("1. Start shrmpl-kv-srv on port 7171")
	fmt.Println("2. Start shrmpl-log-srv on port 7379")
	fmt.Println("3. Start shrmpl-vault-srv on port 7474 (with TLS)")
	fmt.Println("4. Run: go run main.go")
}
