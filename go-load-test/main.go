package main

import (
	"flag"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"
)

type TestConfig struct {
	ServerAddr string
	NumUsers   int
	Operations int
	SharedConn bool
	FullTest   bool
	ConfigFile string
}

type TestResult struct {
	Duration  time.Duration
	Success   bool
	ErrorType string
}

type LoadTest struct {
	config TestConfig
}

func NewLoadTest(config TestConfig) *LoadTest {
	return &LoadTest{config: config}
}

func (lt *LoadTest) Run() []TestResult {
	var results []TestResult

	if lt.config.SharedConn {
		// Shared connection mode (like Golang client)
		results = lt.runSharedConnectionTest()
	} else {
		// Multi-connection mode
		results = lt.runMultiConnectionTest()
	}

	return results
}

func (lt *LoadTest) runSharedConnectionTest() []TestResult {
	// Create ONE shared client that all goroutines will use (simulates Golang client's queuing)
	sharedClient := NewKV(&KVConfig{HostPort: lt.config.ServerAddr})

	var allResults []TestResult
	var resultsMutex sync.Mutex
	var wg sync.WaitGroup

	for userID := 0; userID < lt.config.NumUsers; userID++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			results := lt.runUserTestOnClient(sharedClient, id)
			resultsMutex.Lock()
			allResults = append(allResults, results...)
			resultsMutex.Unlock()
		}(userID)
	}

	wg.Wait()
	sharedClient.Close()
	return allResults
}

func (lt *LoadTest) runMultiConnectionTest() []TestResult {
	var allResults []TestResult
	var wg sync.WaitGroup
	resultsChan := make(chan []TestResult, lt.config.NumUsers)

	for userID := 0; userID < lt.config.NumUsers; userID++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			results := lt.runUserTest(id)
			resultsChan <- results
		}(userID)
	}

	wg.Wait()
	close(resultsChan)

	for results := range resultsChan {
		allResults = append(allResults, results...)
	}

	return allResults
}

func (lt *LoadTest) runUserTest(userID int) []TestResult {
	config := &KVConfig{HostPort: lt.config.ServerAddr}
	client := NewKV(config)
	defer client.Close()

	return lt.runUserTestOnClient(client, userID)
}

func (lt *LoadTest) runUserTestOnClient(client ThisAppKVInterface, userID int) []TestResult {
	var results []TestResult

	for op := 0; op < lt.config.Operations; op++ {
		start := time.Now()

		var success bool
		var err error
		var errorType string

		if lt.config.FullTest {
			// Comprehensive test operations
			success, errorType = lt.runFullTestOperations(client, userID, op)
		} else {
			// Simple batch GET test
			_, err = client.Batch([]string{"GET loginlock-ip-123", "GET loginlock-user-abc"})
			success = err == nil
			if !success {
				errorType = fmt.Sprintf("Batch GET failed: %v", err)
			}
		}

		duration := time.Since(start)
		results = append(results, TestResult{
			Duration:  duration,
			Success:   success,
			ErrorType: errorType,
		})
	}

	return results
}

func (lt *LoadTest) runFullTestOperations(client ThisAppKVInterface, userID, opNum int) (bool, string) {
	key := fmt.Sprintf("test_key_%d_%d", userID, opNum)
	value := fmt.Sprintf("%d", userID)

	// SET operation
	err := client.Set(key, value, "")
	if err != nil {
		return false, fmt.Sprintf("SET failed: %v", err)
	}

	// GET and verify
	gotValue, err := client.Get(key)
	if err != nil {
		return false, fmt.Sprintf("GET failed: %v", err)
	}
	if gotValue != value {
		return false, fmt.Sprintf("GET verification failed: expected %s, got %s", value, gotValue)
	}

	// INCR and verify
	counterKey := fmt.Sprintf("counter_%d", userID)
	count, err := client.Incr(counterKey, "")
	if err != nil {
		return false, fmt.Sprintf("INCR failed: %v", err)
	}
	expectedCount := opNum + 1
	if count != expectedCount {
		return false, fmt.Sprintf("INCR verification failed: expected %d, got %d", expectedCount, count)
	}

	// SET with TTL
	ttlKey := fmt.Sprintf("ttl_key_%d_%d", userID, opNum)
	err = client.Set(ttlKey, "ttl_value", "60s")
	if err != nil {
		return false, fmt.Sprintf("SET with TTL failed: %v", err)
	}

	// Batch GET (always test this)
	_, err = client.Batch([]string{"GET loginlock-ip-123", "GET loginlock-user-abc"})
	if err != nil {
		return false, fmt.Sprintf("Batch GET failed: %v", err)
	}

	return true, ""
}

func (lt *LoadTest) PrintResults(results []TestResult) {
	total := len(results)
	successful := 0
	for _, r := range results {
		if r.Success {
			successful++
		}
	}
	errors := total - successful

	fmt.Println("\nLoad Test Results:")
	fmt.Printf("Total Operations: %d\n", total)
	fmt.Printf("Successful: %d (%.1f%%)\n", successful, float64(successful)/float64(total)*100)
	fmt.Printf("Errors: %d (%.1f%%)\n", errors, float64(errors)/float64(total)*100)

	if errors > 0 {
		errorCounts := make(map[string]int)
		for _, r := range results {
			if r.ErrorType != "" {
				errorCounts[r.ErrorType]++
			}
		}
		fmt.Println("\nError Breakdown:")
		for err, count := range errorCounts {
			fmt.Printf("  %s: %d\n", err, count)
		}
	}

	lt.printTimeDistribution(results, successful)

	fmt.Printf("\nTotal Test Duration: %.2fs\n", time.Since(time.Now().Add(-time.Duration(len(results))*time.Millisecond)).Seconds())
}

func (lt *LoadTest) printTimeDistribution(results []TestResult, successful int) {
	buckets := []time.Duration{10 * time.Millisecond, 50 * time.Millisecond, 100 * time.Millisecond, 200 * time.Millisecond, 500 * time.Millisecond, 1000 * time.Millisecond}
	counts := make([]int, len(buckets)+1)

	for _, r := range results {
		if r.Success {
			found := false
			for i, limit := range buckets {
				if r.Duration < limit {
					counts[i]++
					found = true
					break
				}
			}
			if !found {
				counts[len(counts)-1]++
			}
		}
	}

	fmt.Println("\nResponse Time Distribution (successful operations):")
	fmt.Printf("<10ms: %d (%.1f%%)\n", counts[0], float64(counts[0])/float64(successful)*100)
	fmt.Printf("<50ms: %d (%.1f%%)\n", counts[1], float64(counts[1])/float64(successful)*100)
	fmt.Printf("<100ms: %d (%.1f%%)\n", counts[2], float64(counts[2])/float64(successful)*100)
	fmt.Printf("<200ms: %d (%.1f%%)\n", counts[3], float64(counts[3])/float64(successful)*100)
	fmt.Printf("<500ms: %d (%.1f%%)\n", counts[4], float64(counts[4])/float64(successful)*100)
	fmt.Printf("<1s: %d (%.1f%%)\n", counts[5], float64(counts[5])/float64(successful)*100)
	fmt.Printf(">1s: %d (%.1f%%)\n", counts[6], float64(counts[6])/float64(successful)*100)
}

func loadConfig(configPath string) (string, error) {
	content, err := os.ReadFile(configPath)
	if err != nil {
		return "", fmt.Errorf("failed to read config file: %v", err)
	}

	lines := strings.Split(string(content), "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "BIND_ADDR=") {
			return strings.TrimPrefix(line, "BIND_ADDR="), nil
		}
	}

	return "", fmt.Errorf("BIND_ADDR not found in config")
}

func main() {
	var sharedConn = flag.Bool("multi", false, "Use individual connections per user instead of shared connection")
	var fullTest = flag.Bool("full", false, "Run full comprehensive test")
	flag.Parse()

	args := flag.Args()
	if len(args) != 1 {
		fmt.Fprintf(os.Stderr, "Usage: go-load-test [flags] <config-file>\n")
		fmt.Fprintf(os.Stderr, "Flags:\n")
		flag.PrintDefaults()
		os.Exit(1)
	}

	configFile := args[0]

	serverAddr, err := loadConfig(configFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to load config: %v\n", err)
		os.Exit(1)
	}

	config := TestConfig{
		ServerAddr: serverAddr,
		NumUsers:   5,
		Operations: 10000,
		SharedConn: !*sharedConn, // Default to shared connection mode
		FullTest:   *fullTest,
		ConfigFile: configFile,
	}

	fmt.Println("Load Test Configuration:")
	fmt.Printf("├── Concurrent Users: %d\n", config.NumUsers)
	fmt.Printf("├── Operations per User: %d\n", config.Operations)
	fmt.Printf("├── Total Operations: %d\n", config.NumUsers*config.Operations)
	connMode := "shared"
	if !config.SharedConn {
		connMode = "multi"
	}
	fmt.Printf("├── Connection Mode: %s\n", connMode)
	testMode := "batch GET only"
	if config.FullTest {
		testMode = "full comprehensive"
	}
	fmt.Printf("├── Test Mode: %s\n", testMode)
	fmt.Printf("└── Server: %s\n", config.ServerAddr)
	fmt.Println()
	fmt.Println("Starting test execution...")

	loadTest := NewLoadTest(config)
	results := loadTest.Run()
	loadTest.PrintResults(results)
}
