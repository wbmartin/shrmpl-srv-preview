package shrmpl

import (
	"fmt"
	"net"
	"os"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

// ThisAppLoggerInterface defines the logging interface for this application
type ThisAppLoggerInterface interface {
	Debug(code, message string, keyvals ...interface{})
	Info(code, message string, keyvals ...interface{})
	Warn(code, message string, keyvals ...interface{})
	Error(code, message string, keyvals ...interface{})
	ErrorWithCallerSkip(code, message string, skip int, keyvals ...interface{})
	InfoWithCallerSkip(code, message string, skip int, keyvals ...interface{})
	DebugWithCallerSkip(code, message string, skip int, keyvals ...interface{})
	WarnWithCallerSkip(code, message string, skip int, keyvals ...interface{})
	Close()
}

// Logger wraps shrmpl-log client for structured logging
type Logger struct {
	shrmplLogClient *ShrmplLogClient
	service         string
	hostPort        string
	mu              sync.Mutex
}

// NewLogger creates a logger that uses shrmpl-log
func NewLogger(serverName, logReceiverHostPort string) *Logger {
	fmt.Fprintf(os.Stderr, "DEBUG: Creating shrmpl-log client for %s\n",
		logReceiverHostPort)
	// Create shrmpl-log client internally
	shrmplLogClient, err := NewShrmplLogClient(logReceiverHostPort)
	if err != nil {
		// If we can't create the client, we'll log to console and continue
		// The Log method will handle the case where shrmplLogClient is nil
		fmt.Fprintf(os.Stderr, "Failed to create shrmpl-log client: %s\n",
			err.Error())
		return &Logger{
			shrmplLogClient: nil,
			service:         serverName,
			hostPort:        logReceiverHostPort,
		}
	}

	fmt.Fprintf(os.Stderr, "DEBUG: Connecting to shrmpl-log\n")
	if err := shrmplLogClient.Connect(); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to connect to shrmpl-log: %s\n", err.Error())
		return &Logger{
			shrmplLogClient: nil,
			service:         serverName,
			hostPort:        logReceiverHostPort,
		}
	}
	fmt.Fprintf(os.Stderr, "DEBUG: Connected to shrmpl-log successfully\n")
	return &Logger{
		shrmplLogClient: shrmplLogClient,
		service:         serverName,
		hostPort:        logReceiverHostPort,
	}
}

// log sends a log message to shrmpl-log with caller information
func (l *Logger) log(level string, code string, message string, skip int,
	keyvals ...interface{}) {
	// Parse key-value pairs for username
	username := "unknown"
	for i := 0; i < len(keyvals); i += 2 {
		if i+1 < len(keyvals) && keyvals[i] == "username" {
			if u, ok := keyvals[i+1].(string); ok {
				username = u
			}
		}
	}

	// Format message with username
	formattedMsg := fmt.Sprintf("[%s] %s", username, message)

	// Add caller information with configurable skip
	_, file, line, ok := runtime.Caller(skip)
	callerInfo := ""
	if ok {
		// Extract just the filename from the full path
		parts := strings.Split(file, "/")
		filename := parts[len(parts)-1]
		callerInfo = fmt.Sprintf(" (%s:%d)", filename, line)
	}

	// Append caller info to message
	fullMessage := formattedMsg + callerInfo

	// Ensure connection to shrmpl-log (thread-safe)
	l.mu.Lock()
	if l.shrmplLogClient == nil {
		shrmplLogClient, err := NewShrmplLogClient(l.hostPort)
		if err == nil {
			if err := shrmplLogClient.Connect(); err == nil {
				l.shrmplLogClient = shrmplLogClient
				fmt.Fprintf(os.Stderr, "WARN: Reconnected to shrmpl-log\n")
			}
		}
	}
	shrmplLogClient := l.shrmplLogClient
	l.mu.Unlock()

	// Send to shrmpl-log
	if shrmplLogClient != nil {
		// fmt.Fprintf(os.Stderr, "DEBUG: Sending log to shrmpl-log: [%s] %s\n",
		//	level, fullMessage)
		if err := shrmplLogClient.Log(level, l.service, "0000",
			fullMessage); err != nil {
			fmt.Fprintf(os.Stderr, "ERROR: Failed to send log to shrmpl-log: %s\n",
				err.Error())
			shrmplLogClient.Close()
			// Thread-safe: set to nil while holding lock
			l.mu.Lock()
			if l.shrmplLogClient == shrmplLogClient {
				l.shrmplLogClient = nil
			}
			l.mu.Unlock()
		}
	}

	// Always log to console for local debugging
	fmt.Fprintf(os.Stderr, "[%s] %s: %s\n", level, l.service, fullMessage)
}

// Debug logs at debug level
func (l *Logger) Debug(code, message string, keyvals ...interface{}) {
	l.log("DEBG", code, message, 2, keyvals...)
}

// Info logs at info level
func (l *Logger) Info(code, message string, keyvals ...interface{}) {
	l.log("INFO", code, message, 2, keyvals...)
}

// Warn logs at warn level
func (l *Logger) Warn(code, message string, keyvals ...interface{}) {
	l.log("WARN", code, message, 2, keyvals...)
}

// Error logs at error level
func (l *Logger) Error(code, message string, keyvals ...interface{}) {
	l.log("ERRO", code, message, 2, keyvals...)
}

// ErrorWithCallerSkip logs at error level with custom caller skip level
func (l *Logger) ErrorWithCallerSkip(
	code, message string,
	skip int,
	keyvals ...interface{},
) {
	l.log("ERRO", code, message, skip, keyvals...)
}

// InfoWithCallerSkip logs at info level with custom caller skip level
func (l *Logger) InfoWithCallerSkip(
	code, message string,
	skip int,
	keyvals ...interface{},
) {
	l.log("INFO", code, message, skip, keyvals...)
}

// DebugWithCallerSkip logs at debug level with custom caller skip level
func (l *Logger) DebugWithCallerSkip(
	code, message string,
	skip int,
	keyvals ...interface{},
) {
	l.log("DEBG", code, message, skip, keyvals...)
}

// WarnWithCallerSkip logs at warn level with custom caller skip level
func (l *Logger) WarnWithCallerSkip(
	code, message string,
	skip int,
	keyvals ...interface{},
) {
	l.log("WARN", code, message, skip, keyvals...)
}

// Close closes the underlying log client connection
func (l *Logger) Close() {
	if l.shrmplLogClient != nil {
		l.shrmplLogClient.Close()
	}
}

// ShrmplLogClient represents a client for the shrmpl-log service
type ShrmplLogClient struct {
	host string
	port int
	conn net.Conn
}

// NewShrmplLogClient creates a new shrmpl-log client
func NewShrmplLogClient(logDest string) (*ShrmplLogClient, error) {
	host, portStr, err := net.SplitHostPort(logDest)
	if err != nil {
		return nil, fmt.Errorf("invalid log destination format: %s", logDest)
	}

	port, err := strconv.Atoi(portStr)
	if err != nil {
		return nil, fmt.Errorf("invalid port in log destination: %w", err)
	}

	return &ShrmplLogClient{
		host: host,
		port: port,
	}, nil
}

// Connect establishes connection to shrmpl-log
func (c *ShrmplLogClient) Connect() error {
	addr := net.JoinHostPort(c.host, strconv.Itoa(c.port))
	conn, err := net.DialTimeout("tcp", addr, 5*time.Second)
	if err != nil {
		return fmt.Errorf("failed to connect to shrmpl-log: %w", err)
	}

	if tcpConn, ok := conn.(*net.TCPConn); ok {
		_ = tcpConn.SetNoDelay(true)
	}

	c.conn = conn
	return nil
}

// Log sends a log message to shrmpl-log
func (c *ShrmplLogClient) Log(level, host, code, message string) error {
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

	// Format: [LVL(4)] [HOST(32)] [CODE(12)] [LEN(5)]: [MSG]\n
	paddedHost := fmt.Sprintf("%-32s", host[:min(32, len(host))])
	paddedLevel := fmt.Sprintf("%-4s", level[:4])
	paddedCode := fmt.Sprintf("%-12s", code[:min(12, len(code))])
	msgLen := fmt.Sprintf("%05d", len(message))

	logLine := fmt.Sprintf("%s %s %s %s: %s\n", paddedLevel, paddedHost, paddedCode, msgLen, message)

	_, err := c.conn.Write([]byte(logLine))
	return err
}

// Close closes the connection to shrmpl-log
func (c *ShrmplLogClient) Close() {
	if c.conn != nil {
		c.conn.Close()
		c.conn = nil
	}
}

// min returns the minimum of two integers
func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
