package shrmpl

import (
	"bufio"
	"crypto/tls"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// KV Client
type KVClient struct {
	host string
	port int
	conn net.Conn
}

// KVListItem represents a key-value pair with optional expiration
type KVListItem struct {
	Key       string
	Value     string
	ExpiresAt *int64 // Unix timestamp, nil if no expiration
}

func NewKVClient(host string, port int) *KVClient {
	return &KVClient{
		host: host,
		port: port,
	}
}

func (c *KVClient) Connect() (bool, error) {
	addr := fmt.Sprintf("%s:%d", c.host, c.port)
	conn, err := net.DialTimeout("tcp", addr, 5*time.Second)
	if err != nil {
		return false, err
	}
	
	// Set TCP_NODELAY
	if tcpConn, ok := conn.(*net.TCPConn); ok {
		tcpConn.SetNoDelay(true)
	}
	
	c.conn = conn
	return true, nil
}

func (c *KVClient) sendCommand(cmd string) (string, error) {
	if c.conn == nil {
		return "", fmt.Errorf("not connected")
	}
	
	// Send command
	_, err := c.conn.Write([]byte(cmd + "\n"))
	if err != nil {
		return "", err
	}
	
	// Read response
	reader := bufio.NewReader(c.conn)
	response, err := reader.ReadString('\n')
	if err != nil {
		return "", err
	}
	
	response = strings.TrimSpace(response)
	
	// Filter out heartbeats
	if response == "UPONG" {
		return "", fmt.Errorf("heartbeat received")
	}
	if response == "TERM" {
		return "", fmt.Errorf("server shutting down")
	}
	
	return response, nil
}

func (c *KVClient) Get(key string) (string, error) {
	if len(key) > 100 {
		return "", fmt.Errorf("key length exceeds 100 characters")
	}
	
	response, err := c.sendCommand(fmt.Sprintf("GET %s", key))
	if err != nil {
		if strings.Contains(err.Error(), "key not found") {
			return "", nil // Key not found is not an error
		}
		return "", err
	}
	
	if strings.HasPrefix(response, "ERROR") {
		return "", fmt.Errorf(response)
	}
	
	return response, nil
}

func (c *KVClient) Set(key, value string, ttl string) (bool, error) {
	if len(key) > 100 || len(value) > 100 {
		return false, fmt.Errorf("key or value length exceeds 100 characters")
	}
	
	var cmd string
	if ttl != "" {
		cmd = fmt.Sprintf("SET %s %s %s", key, value, ttl)
	} else {
		cmd = fmt.Sprintf("SET %s %s", key, value)
	}
	
	response, err := c.sendCommand(cmd)
	if err != nil {
		return false, err
	}
	
	return response == "OK", nil
}

func (c *KVClient) Incr(key string, ttl string) (int, error) {
	if len(key) > 100 {
		return 0, fmt.Errorf("key length exceeds 100 characters")
	}
	
	var cmd string
	if ttl != "" {
		cmd = fmt.Sprintf("INCR %s %s", key, ttl)
	} else {
		cmd = fmt.Sprintf("INCR %s", key)
	}
	
	response, err := c.sendCommand(cmd)
	if err != nil {
		return 0, err
	}
	
	if strings.HasPrefix(response, "ERROR") {
		return 0, fmt.Errorf(response)
	}
	
	result, err := strconv.Atoi(response)
	if err != nil {
		return 0, fmt.Errorf("invalid response: %s", response)
	}
	
	return result, nil
}

func (c *KVClient) Delete(key string) (bool, error) {
	if len(key) > 100 {
		return false, fmt.Errorf("key length exceeds 100 characters")
	}
	
	response, err := c.sendCommand(fmt.Sprintf("DEL %s", key))
	if err != nil {
		if strings.Contains(err.Error(), "key not found") {
			return false, nil // Key not found is not an error
		}
		return false, err
	}
	
	return response == "OK", nil
}

func (c *KVClient) Ping() (bool, error) {
	response, err := c.sendCommand("PING")
	if err != nil {
		return false, err
	}
	
	return response == "PONG", nil
}

func (c *KVClient) List() ([]KVListItem, error) {
	response, err := c.sendCommand("LIST")
	if err != nil {
		return nil, err
	}
	
	if strings.HasPrefix(response, "ERROR") {
		return nil, fmt.Errorf(response)
	}
	
	var items []KVListItem
	if strings.TrimSpace(response) == "" {
		return items, nil
	}
	
	lines := strings.Split(response, "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		
		// Parse format: key=value,expiration
		parts := strings.SplitN(line, "=", 3)
		if len(parts) != 3 {
			continue
		}
		
		key := parts[0]
		valueAndExpiration := parts[2]
		
		// Split value and expiration
		valueParts := strings.SplitN(valueAndExpiration, ",", 2)
		if len(valueParts) != 2 {
			continue
		}
		
		value := valueParts[0]
		expirationStr := valueParts[1]
		
		var expiresAt *int64
		if expirationStr != "no-expiration" {
			if timestamp, err := strconv.ParseInt(expirationStr, 10, 64); err == nil {
				expiresAt = &timestamp
			}
		}
		
		items = append(items, KVListItem{
			Key:       key,
			Value:     value,
			ExpiresAt: expiresAt,
		})
	}
	
	return items, nil
}

func (c *KVClient) Close() {
	if c.conn != nil {
		c.conn.Close()
		c.conn = nil
	}
}

// Log Client
type LogClient struct {
	host string
	port int
	conn net.Conn
}

func NewLogClient(host string, port int) *LogClient {
	return &LogClient{
		host: host,
		port: port,
	}
}

func (c *LogClient) Connect() (bool, error) {
	addr := fmt.Sprintf("%s:%d", c.host, c.port)
	conn, err := net.DialTimeout("tcp", addr, 5*time.Second)
	if err != nil {
		return false, err
	}
	
	// Set TCP_NODELAY
	if tcpConn, ok := conn.(*net.TCPConn); ok {
		tcpConn.SetNoDelay(true)
	}
	
	c.conn = conn
	return true, nil
}

func (c *LogClient) Send(level, host, code, message string) error {
	// Validate inputs
	if len(level) != 4 {
		return fmt.Errorf("level must be exactly 4 characters")
	}
	if len(host) > 32 {
		return fmt.Errorf("host must be <= 32 characters")
	}
	if len(code) != 4 {
		return fmt.Errorf("code must be exactly 4 characters")
	}
	if len(message) > 4096 {
		return fmt.Errorf("message must be <= 4096 characters")
	}
	
	// Format: [LVL(4)] [HOST(32)] [CODE(4)] [LEN(4)]: [MSG]\n
	paddedHost := fmt.Sprintf("%-32s", host[:32])
	paddedLevel := fmt.Sprintf("%-4s", level[:4])
	paddedCode := fmt.Sprintf("%-4s", code[:4])
	msgLen := fmt.Sprintf("%04d", len(message))
	
	logLine := fmt.Sprintf("[%s] [%s] [%s] [%s]: %s\n", paddedLevel, paddedHost, paddedCode, msgLen, message)
	
	_, err := c.conn.Write([]byte(logLine))
	return err
}

func (c *LogClient) Close() {
	if c.conn != nil {
		c.conn.Close()
		c.conn = nil
	}
}

// Vault Client
type VaultClient struct {
	serverURL string
	certPath  string
	keyPath   string
	secret    string
	client    *http.Client
}

func NewVaultClient(serverURL, certPath, keyPath, secret string) *VaultClient {
	return &VaultClient{
		serverURL: strings.TrimRight(serverURL, "/"),
		certPath:  certPath,
		keyPath:   keyPath,
		secret:    secret,
	}
}

func (c *VaultClient) Connect() (bool, error) {
	// Load client certificates
	cert, err := tls.LoadX509KeyPair(c.certPath, c.keyPath)
	if err != nil {
		return false, fmt.Errorf("failed to load certificates: %v", err)
	}
	
	// Create TLS config
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
	}
	
	// Create HTTP client
	transport := &http.Transport{
		TLSClientConfig: tlsConfig,
	}
	c.client = &http.Client{
		Transport: transport,
		Timeout:   10 * time.Second,
	}
	
	// Connection setup successful - actual testing happens during GetConfig calls
	return true, nil
}

func (c *VaultClient) GetConfig(filename string) (string, error) {
	if c.client == nil {
		return "", fmt.Errorf("not connected")
	}
	
	url := fmt.Sprintf("%s/%s?secret=%s", c.serverURL, filename, c.secret)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	
	resp, err := c.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	
	switch resp.StatusCode {
	case 200:
		content, err := io.ReadAll(resp.Body)
		return string(content), err
	case 404:
		return "", fmt.Errorf("file not found")
	case 401:
		return "", fmt.Errorf("unauthorized - invalid certificate or secret")
	case 429:
		return "", fmt.Errorf("rate limit exceeded")
	default:
		return "", fmt.Errorf("HTTP error: %d", resp.StatusCode)
	}
}