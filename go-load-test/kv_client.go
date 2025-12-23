package main

import (
	"bufio"
	"errors"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

// ThisAppKVInterface defines the key-value store interface for this application
type ThisAppKVInterface interface {
	Get(key string) (string, error)
	Set(key, value, ttl string) error
	Incr(key string, ttl string) (int, error)
	Batch(commands []string) ([]string, error)
	Close()
}

// KV wraps shrmpl-kv client for key-value operations
type KV struct {
	shrmplKVClient *ShrmplKVClient
	hostPort       string
	mu             sync.Mutex
}

// parseHostPort parses a "host:port" string into separate
// host and port components
func parseHostPort(hostPort string) (string, string, error) {
	host, port, err := net.SplitHostPort(hostPort)
	if err != nil {
		return "", "", fmt.Errorf("invalid host:port format: %s", hostPort)
	}
	return host, port, nil
}

// NewKV creates a key-value store client
func NewKV(config *KVConfig) ThisAppKVInterface {
	// Parse the combined host:port string
	host, portStr, err := parseHostPort(config.HostPort)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to parse kv_host_port: %s\n", err.Error())
		return &KV{shrmplKVClient: nil, hostPort: config.HostPort}
	}

	port, err := strconv.Atoi(portStr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Invalid port in kv_host_port: %s\n", err.Error())
		return &KV{shrmplKVClient: nil, hostPort: config.HostPort}
	}

	shrmplKV := NewShrmplKVClient(host, port)
	if err := shrmplKV.Connect(); err != nil {
		// If we can't connect, we'll return a client that logs errors
		// The operations will fail gracefully
		fmt.Fprintf(os.Stderr, "Failed to connect to shrmpl-kv: %s\n", err.Error())
		return &KV{shrmplKVClient: nil, hostPort: config.HostPort}
	}

	return &KV{
		shrmplKVClient: shrmplKV,
		hostPort:       config.HostPort,
	}
}

// tryReconnect attempts to reconnect to the KV server
func (kv *KV) tryReconnect() {
	host, portStr, err := parseHostPort(kv.hostPort)
	if err != nil {
		return
	}
	port, err := strconv.Atoi(portStr)
	if err != nil {
		return
	}
	client := NewShrmplKVClient(host, port)
	if err := client.Connect(); err == nil {
		kv.shrmplKVClient = client
	}
}

// Get retrieves a value from the key-value store
func (kv *KV) Get(key string) (string, error) {
	kv.mu.Lock()
	defer kv.mu.Unlock()

	if kv.shrmplKVClient == nil {
		kv.tryReconnect()
	}
	if kv.shrmplKVClient == nil {
		return "", fmt.Errorf("key-value store not available")
	}

	val, err := kv.shrmplKVClient.Get(key)
	if err != nil {
		kv.shrmplKVClient.Close()
		kv.shrmplKVClient = nil
		return "", err
	}
	return val, nil
}

// Set stores a key-value pair with optional TTL
func (kv *KV) Set(key, value, ttl string) error {
	kv.mu.Lock()
	defer kv.mu.Unlock()

	if kv.shrmplKVClient == nil {
		kv.tryReconnect()
	}
	if kv.shrmplKVClient == nil {
		return fmt.Errorf("key-value store not available")
	}

	err := kv.shrmplKVClient.Set(key, value, ttl)
	if err != nil {
		kv.shrmplKVClient.Close()
		kv.shrmplKVClient = nil
		return err
	}
	return nil
}

// Incr increments a counter and returns the new value
func (kv *KV) Incr(key string, ttl string) (int, error) {
	kv.mu.Lock()
	defer kv.mu.Unlock()

	if kv.shrmplKVClient == nil {
		kv.tryReconnect()
	}
	if kv.shrmplKVClient == nil {
		return 0, fmt.Errorf("key-value store not available")
	}

	val, err := kv.shrmplKVClient.Incr(key, ttl)
	if err != nil {
		kv.shrmplKVClient.Close()
		kv.shrmplKVClient = nil
		return 0, err
	}
	return val, nil
}

// Batch executes multiple commands in a single call
func (kv *KV) Batch(commands []string) ([]string, error) {
	if len(commands) > 3 {
		return nil, fmt.Errorf("batch cannot exceed 3 commands")
	}

	kv.mu.Lock()
	defer kv.mu.Unlock()

	if kv.shrmplKVClient == nil {
		kv.tryReconnect()
	}
	if kv.shrmplKVClient == nil {
		return nil, fmt.Errorf("key-value store not available")
	}

	batchCmd := "BATCH " + strings.Join(commands, ";")
	response, err := kv.shrmplKVClient.sendCommand(batchCmd)
	if err != nil {
		kv.shrmplKVClient.Close()
		kv.shrmplKVClient = nil
		return nil, err
	}

	if strings.HasPrefix(response, "ERROR") {
		return nil, errors.New(response)
	}

	results := strings.Split(strings.TrimSpace(response), ";")
	return results, nil
}

// Close closes the underlying KV client connection
func (kv *KV) Close() {
	kv.mu.Lock()
	defer kv.mu.Unlock()
	if kv.shrmplKVClient != nil {
		kv.shrmplKVClient.Close()
		kv.shrmplKVClient = nil
	}
}

// ShrmplKVClient represents a client for the shrmpl-kv service
type ShrmplKVClient struct {
	host    string
	port    int
	conn    net.Conn
	timeout time.Duration
}

// NewShrmplKVClient creates a new shrmpl-kv client
func NewShrmplKVClient(host string, port int) *ShrmplKVClient {
	return &ShrmplKVClient{
		host:    host,
		port:    port,
		timeout: 5 * time.Second,
	}
}

// Connect establishes connection to shrmpl-kv
func (c *ShrmplKVClient) Connect() error {
	addr := net.JoinHostPort(c.host, strconv.Itoa(c.port))
	conn, err := net.DialTimeout("tcp", addr, 5*time.Second)
	if err != nil {
		return fmt.Errorf("failed to connect to shrmpl-kv: %w", err)
	}

	if tcpConn, ok := conn.(*net.TCPConn); ok {
		_ = tcpConn.SetNoDelay(true)
		_ = tcpConn.SetReadDeadline(time.Now().Add(c.timeout))
	}

	c.conn = conn
	return nil
}

// Get retrieves a value from shrmpl-kv
func (c *ShrmplKVClient) Get(key string) (string, error) {
	if len(key) > 100 {
		return "", fmt.Errorf("key length exceeds 100 characters")
	}

	response, err := c.sendCommand(fmt.Sprintf("GET %s", key))
	if err != nil {
		return "", err
	}

	if response == "*KEY NOT FOUND*" {
		return "", nil
	}
	if strings.HasPrefix(response, "ERROR") {
		return "", errors.New(response)
	}

	return response, nil
}

// Set stores a key-value pair in shrmpl-kv
func (c *ShrmplKVClient) Set(key, value string, ttl string) error {
	if len(key) > 100 || len(value) > 100 {
		return fmt.Errorf("key or value length exceeds 100 characters")
	}

	var cmd string
	if ttl != "" {
		cmd = fmt.Sprintf("SET %s %s %s", key, value, ttl)
	} else {
		cmd = fmt.Sprintf("SET %s %s", key, value)
	}

	response, err := c.sendCommand(cmd)
	if err != nil {
		return err
	}

	if response != "OK" {
		return fmt.Errorf("unexpected response: %s", response)
	}

	return nil
}

// Incr increments a counter in shrmpl-kv
func (c *ShrmplKVClient) Incr(key string, ttl string) (int, error) {
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
		return 0, errors.New(response)
	}

	result, err := strconv.Atoi(response)
	if err != nil {
		return 0, fmt.Errorf("invalid response: %s", response)
	}

	return result, nil
}

// Close closes the connection to shrmpl-kv
func (c *ShrmplKVClient) Close() {
	if c == nil || c.conn == nil {
		return
	}
	c.conn.Close()
	c.conn = nil
}

// sendCommand sends a command and returns the response
func (c *ShrmplKVClient) sendCommand(cmd string) (string, error) {
	if c.conn == nil {
		return "", fmt.Errorf("not connected")
	}

	// Set read deadline for this operation
	if tcpConn, ok := c.conn.(*net.TCPConn); ok {
		_ = tcpConn.SetReadDeadline(time.Now().Add(c.timeout))
	}

	_, err := c.conn.Write([]byte(cmd + "\n"))
	if err != nil {
		return "", err
	}

	reader := bufio.NewReader(c.conn)
	for {
		response, err := reader.ReadString('\n')
		if err != nil {
			return "", err
		}

		response = strings.TrimSpace(response)

		// Skip heartbeats
		if response == "UPONG" {
			continue
		}
		if response == "TERM" {
			return "", fmt.Errorf("server shutting down")
		}

		return response, nil
	}
}

// KVConfig for configuring the KV client
type KVConfig struct {
	HostPort string
}
