#!/usr/bin/env python3
"""
Example usage of Shrmpl client library

Demonstrates KV, Log, and Vault service usage.
Configuration values are hardcoded to focus on library usage.
"""

import sys
import os
sys.path.append(os.path.dirname(os.path.abspath(__file__)))

from shrmpl import ShrmplKV, ShrmplLog, ShrmplVault


def main():
    print("=== Shrmpl Client Library Example ===\n")
    
    # KV Server Example
    print("1. KV Server Example:")
    kv = ShrmplKV("127.0.0.1", 7171)
    
    success, error = kv.connect()
    if not success:
        print(f"   KV connect failed: {error}")
        return
    print("   ✓ Connected to KV server")
    
    # Test SET with TTL
    success, error = kv.set("example_key", "example_value", "30s")
    if success:
        print("   ✓ SET example_key = example_value (30s TTL)")
    else:
        print(f"   ✗ SET failed: {error}")
    
    # Test GET
    value, error = kv.get("example_key")
    if error is None:
        print(f"   ✓ GET example_key = {value}")
    else:
        print(f"   ✗ GET failed: {error}")
    
    # Test INCR
    count, error = kv.incr("counter", "1min")
    if error is None:
        print(f"   ✓ INCR counter = {count}")
    else:
        print(f"   ✗ INCR failed: {error}")
    
    # Test LIST
    items, error = kv.list()
    if error is None:
        print("   ✓ LIST successful")
        if items:
            print("   Current keys:")
            for key, value, expiration in items:
                if expiration:
                    import datetime
                    exp_time = datetime.datetime.fromtimestamp(expiration)
                    print(f"     {key} = {value} (expires: {exp_time})")
                else:
                    print(f"     {key} = {value} (no expiration)")
        else:
            print("   (no keys stored)")
    else:
        print(f"   ✗ LIST failed: {error}")
    
    # Test PING
    success, error = kv.ping()
    if success:
        print("   ✓ PING successful")
    else:
        print(f"   ✗ PING failed: {error}")
    
    kv.close()
    print()
    
    # Log Server Example
    print("2. Log Server Example:")
    log = ShrmplLog("127.0.0.1", 7379)
    
    success, error = log.connect()
    if not success:
        print(f"   Log connect failed: {error}")
        return
    print("   ✓ Connected to Log server")
    
    # Test log sending
    success, error = log.send("INFO", "example-host", "T001", "Application started successfully")
    if success:
        print("   ✓ Sent INFO log message")
    else:
        print(f"   ✗ Log send failed: {error}")
    
    success, error = log.send("ERRO", "example-host", "E001", "Database connection failed")
    if success:
        print("   ✓ Sent ERRO log message")
    else:
        print(f"   ✗ Log send failed: {error}")
    
    log.close()
    print()
    
    # Vault Server Example
    print("3. Vault Server Example:")
    vault = ShrmplVault(
        server_url="https://127.0.0.1:7474",
        cert_path="/path/to/client.crt",
        key_path="/path/to/client.key",
        secret="example_secret"
    )
    
    success, error = vault.connect()
    if not success:
        print(f"   Vault connect failed: {error}")
        print("   Note: This example requires actual certificates and running vault server")
    else:
        print("   ✓ Connected to Vault server")
        
        # Test config retrieval
        content, error = vault.get_config("example-config-file")
        if error is None:
            print("   ✓ Retrieved config file")
            if content and len(content) > 100:
                print(f"   Content preview: {content[:100]}...")
            else:
                print(f"   Content: {content}")
        else:
            print(f"   ✗ Config retrieval failed: {error}")
    
    print("\n=== Example Complete ===")
    print("\nTo run this example:")
    print("1. Start shrmpl-kv-srv on port 7171")
    print("2. Start shrmpl-log-srv on port 7379") 
    print("3. Start shrmpl-vault-srv on port 7474 (with TLS)")
    print("4. Run: python3 example.py")


if __name__ == "__main__":
    main()