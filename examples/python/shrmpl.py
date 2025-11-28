#!/usr/bin/env python3
"""
Shrmpl Client Library

Provides client classes for KV, Log, and Vault services.
Each service maintains persistent connections and returns (result, error) tuples.
"""

import socket
import ssl
import urllib.request
import urllib.error
from typing import Optional, Tuple, Union


class ShrmplKV:
    """Client for Shrmpl KV Server"""
    
    def __init__(self, host: str, port: int):
        self.host = host
        self.port = port
        self.socket = None
        self.reader = None
        self.writer = None
    
    def connect(self) -> Tuple[bool, Optional[str]]:
        """Connect to KV server"""
        try:
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.settimeout(5)
            self.socket.connect((self.host, self.port))
            self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            return True, None
        except Exception as e:
            return False, str(e)
    
    def _send_command(self, command: str) -> Tuple[Optional[str], Optional[str]]:
        """Send command and get response"""
        if not self.socket:
            return None, "Not connected"
        
        try:
            # Send command
            self.socket.sendall(f"{command}\n".encode())
            
            # Read response
            response = ""
            while True:
                chunk = self.socket.recv(1024).decode()
                if not chunk:
                    break
                response += chunk
                if response.endswith('\n'):
                    break
            
            response = response.strip()
            
            # Filter out heartbeats
            if response == "UPONG":
                return None, "Heartbeat received"
            elif response == "TERM":
                return None, "Server shutting down"
            elif response.startswith("ERROR"):
                return None, response
            else:
                return response, None
                
        except Exception as e:
            return None, str(e)
    
    def get(self, key: str) -> Tuple[Optional[str], Optional[str]]:
        """Get value for key"""
        if len(key) > 100:
            return None, "Key length exceeds 100 characters"
        
        result, error = self._send_command(f"GET {key}")
        if error:
            if "key not found" in str(error):
                return None, None  # Key not found is not an error
            return None, error
        return result, None
    
    def set(self, key: str, value: str, ttl: Optional[str] = None) -> Tuple[bool, Optional[str]]:
        """Set key to value with optional TTL"""
        if len(key) > 100 or len(value) > 100:
            return False, "Key or value length exceeds 100 characters"
        
        if ttl:
            command = f"SET {key} {value} {ttl}"
        else:
            command = f"SET {key} {value}"
        
        result, error = self._send_command(command)
        if error:
            return False, error
        return result == "OK", None
    
    def incr(self, key: str, ttl: Optional[str] = None) -> Tuple[Optional[int], Optional[str]]:
        """Increment integer value by 1 with optional TTL"""
        if len(key) > 100:
            return None, "Key length exceeds 100 characters"
        
        if ttl:
            command = f"INCR {key} {ttl}"
        else:
            command = f"INCR {key}"
        
        result, error = self._send_command(command)
        if error:
            return None, error
        
        if result is None:
            return None, "No response received"
        
        try:
            return int(result), None
        except (ValueError, TypeError):
            return None, f"Invalid response: {result}"
    
    def delete(self, key: str) -> Tuple[bool, Optional[str]]:
        """Delete key"""
        if len(key) > 100:
            return False, "Key length exceeds 100 characters"
        
        result, error = self._send_command(f"DEL {key}")
        if error:
            if "key not found" in str(error):
                return False, None  # Key not found is not an error
            return False, error
        return result == "OK", None
    
    def ping(self) -> Tuple[bool, Optional[str]]:
        """Ping server"""
        result, error = self._send_command("PING")
        if error:
            return False, error
        return result == "PONG", None
    
    def list(self) -> Tuple[Optional[list], Optional[str]]:
        """List all keys with values and expiration timestamps"""
        result, error = self._send_command("LIST")
        if error:
            return None, error
        
        if result is None:
            return [], None
        
        items = []
        if result.strip() == "":
            return [], None
        
        for line in result.strip().split('\n'):
            if line.strip() == "":
                continue
            
            parts = line.split('=', 2)
            if len(parts) != 3:
                continue
            
            key = parts[0]
            value_and_expiration = parts[2]
            value_parts = value_and_expiration.split(',', 1)
            
            if len(value_parts) != 2:
                continue
            
            value = value_parts[0]
            expiration_str = value_parts[1]
            
            if expiration_str == "no-expiration":
                expiration = None
            else:
                try:
                    expiration = int(expiration_str)
                except ValueError:
                    expiration = None
            
            items.append((key, value, expiration))
        
        return items, None
    
    def close(self):
        """Close connection"""
        if self.socket:
            self.socket.close()
            self.socket = None


class ShrmplLog:
    """Client for Shrmpl Log Server"""
    
    def __init__(self, host: str, port: int):
        self.host = host
        self.port = port
        self.socket = None
    
    def connect(self) -> Tuple[bool, Optional[str]]:
        """Connect to Log server"""
        try:
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.settimeout(5)
            self.socket.connect((self.host, self.port))
            self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            return True, None
        except Exception as e:
            return False, str(e)
    
    def send(self, level: str, host: str, code: str, message: str) -> Tuple[bool, Optional[str]]:
        """Send log message"""
        try:
            # Validate inputs
            if len(level) != 4:
                return False, "Level must be exactly 4 characters"
            if len(host) > 32:
                return False, "Host must be <= 32 characters"
            if len(code) != 4:
                return False, "Code must be exactly 4 characters"
            if len(message) > 4096:
                return False, "Message must be <= 4096 characters"
            
            # Format: [LVL(4)] [HOST(32)] [CODE(4)] [LEN(4)]: [MSG]\n
            padded_host = host.ljust(32)[:32]  # Pad or truncate to 32 chars
            padded_level = level.ljust(4)[:4]    # Pad or truncate to 4 chars
            padded_code = code.ljust(4)[:4]      # Pad or truncate to 4 chars
            msg_len = f"{len(message):04d}"     # Zero-padded length
            
            log_line = f"[{padded_level}] [{padded_host}] [{padded_code}] [{msg_len}]: {message}\n"
            
            if self.socket:
                self.socket.sendall(log_line.encode())
            return True, None
            
        except Exception as e:
            return False, str(e)
    
    def close(self):
        """Close connection"""
        if self.socket:
            self.socket.close()
            self.socket = None


class ShrmplVault:
    """Client for Shrmpl Vault Server"""
    
    def __init__(self, server_url: str, cert_path: str, key_path: str, secret: str):
        self.server_url = server_url.rstrip('/')
        self.cert_path = cert_path
        self.key_path = key_path
        self.secret = secret
    
    def connect(self) -> Tuple[bool, Optional[str]]:
        """Validate certificates and test connection"""
        try:
            # Create SSL context
            context = ssl.create_default_context()
            context.check_hostname = False
            context.verify_mode = ssl.CERT_REQUIRED
            
            # Load client certificates
            context.load_cert_chain(self.cert_path, self.key_path)
            
            # Test connection with a simple request
            test_url = f"{self.server_url}/test?secret={self.secret}"
            req = urllib.request.Request(test_url)
            
            try:
                with urllib.request.urlopen(req, context=context, timeout=10) as response:
                    if response.status == 404:
                        # 404 is expected for test file, means connection works
                        return True, None
                    else:
                        return False, f"Test failed with status: {response.status}"
            except urllib.error.HTTPError as e:
                if e.code == 404:
                    # 404 is expected for test file
                    return True, None
                else:
                    return False, f"HTTP error: {e.code} - {e.reason}"
                    
        except Exception as e:
            return False, str(e)
    
    def get_config(self, filename: str) -> Tuple[Optional[str], Optional[str]]:
        """Get configuration file from vault"""
        try:
            # Create SSL context
            context = ssl.create_default_context()
            context.check_hostname = False
            context.verify_mode = ssl.CERT_REQUIRED
            context.load_cert_chain(self.cert_path, self.key_path)
            
            # Make request
            url = f"{self.server_url}/{filename}?secret={self.secret}"
            req = urllib.request.Request(url)
            
            with urllib.request.urlopen(req, context=context, timeout=10) as response:
                if response.status == 200:
                    content = response.read().decode()
                    return content, None
                elif response.status == 404:
                    return None, "File not found"
                elif response.status == 401:
                    return None, "Unauthorized - invalid certificate or secret"
                elif response.status == 429:
                    return None, "Rate limit exceeded"
                else:
                    return None, f"HTTP error: {response.status}"
                    
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None, "File not found"
            elif e.code == 401:
                return None, "Unauthorized - invalid certificate or secret"
            elif e.code == 429:
                return None, "Rate limit exceeded"
            else:
                return None, f"HTTP error: {e.code} - {e.reason}"
        except Exception as e:
            return None, str(e)