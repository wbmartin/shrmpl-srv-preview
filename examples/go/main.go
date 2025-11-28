package main

import (
	"fmt"
	"time"

	"shrmpl"
)

func main() {
	fmt.Println("=== Shrmpl Client Library Example ===\n")

	// KV Server Example
	fmt.Println("1. KV Server Example:")
	kv := shrmpl.NewKVClient("127.0.0.1", 7171)

	success, err := kv.Connect()
	if !success {
		fmt.Printf("   KV connect failed: %v\n", err)
		return
	}
	fmt.Println("   ✓ Connected to KV server")

	// Test SET with TTL
	success, err = kv.Set("example_key", "example_value", "30s")
	if success {
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

	// Test LIST
	items, err := kv.List()
	if err == nil {
		fmt.Println("   ✓ LIST successful")
		if len(items) > 0 {
			fmt.Println("   Current keys:")
			for _, item := range items {
				if item.ExpiresAt != nil {
					expTime := time.Unix(*item.ExpiresAt, 0)
					fmt.Printf("     %s = %s (expires: %v)\n", item.Key, item.Value, expTime)
				} else {
					fmt.Printf("     %s = %s (no expiration)\n", item.Key, item.Value)
				}
			}
		} else {
			fmt.Println("   (no keys stored)")
		}
	} else {
		fmt.Printf("   ✗ LIST failed: %v\n", err)
	}

	// Test PING
	success, err = kv.Ping()
	if success {
		fmt.Println("   ✓ PING successful")
	} else {
		fmt.Printf("   ✗ PING failed: %v\n", err)
	}

	kv.Close()
	fmt.Println()

	// Log Server Example
	fmt.Println("2. Log Server Example:")
	logClient := shrmpl.NewLogClient("127.0.0.1", 7379)

	success, err = logClient.Connect()
	if !success {
		fmt.Printf("   Log connect failed: %v\n", err)
		return
	}
	fmt.Println("   ✓ Connected to Log server")

	// Test log sending
	err = logClient.Send("INFO", "example-host", "T001", "Application started successfully")
	if err == nil {
		fmt.Println("   ✓ Sent INFO log message")
	} else {
		fmt.Printf("   ✗ Log send failed: %v\n", err)
	}

	err = logClient.Send("ERRO", "example-host", "E001", "Database connection failed")
	if err == nil {
		fmt.Println("   ✓ Sent ERRO log message")
	} else {
		fmt.Printf("   ✗ Log send failed: %v\n", err)
	}

	logClient.Close()
	fmt.Println()

	// Vault Server Example
	fmt.Println("3. Vault Server Example:")
	vault := shrmpl.NewVaultClient(
		"https://127.0.0.1:7474",
		"/path/to/client.crt",
		"/path/to/client.key",
		"example_secret",
	)

	success, err = vault.Connect()
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
	fmt.Println("4. Run: go run example.go")
}