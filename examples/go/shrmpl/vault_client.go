package shrmpl

import (
	"crypto/tls"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// VaultClient represents a client for the shrmpl-vault service
type VaultClient struct {
	serverURL string
	certPath  string
	keyPath   string
	secret    string
	client    *http.Client
}

// NewVaultClient creates a new vault client
func NewVaultClient(serverURL, certPath, keyPath, secret string) *VaultClient {
	return &VaultClient{
		serverURL: strings.TrimRight(serverURL, "/"),
		certPath:  certPath,
		keyPath:   keyPath,
		secret:    secret,
	}
}

// Connect establishes TLS connection to shrmpl-vault
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

// GetConfig retrieves a configuration file from shrmpl-vault
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
