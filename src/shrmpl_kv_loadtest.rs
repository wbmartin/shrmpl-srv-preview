use clap::{Arg, Command};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration, Instant};

mod shrmpl_kv_client;
use shrmpl_kv_client::KvClient;

#[derive(Clone)]
struct TestConfig {
    server_addr: String,
    num_users: usize,
    operations_per_user: usize,
    shared_connection: bool,
    full_test: bool,
}

#[derive(Debug, Clone)]
struct TestResult {
    duration: Duration,
    success: bool,
    error_type: Option<String>,
}

async fn run_test(config: TestConfig) -> Result<Vec<TestResult>, String> {
    let mut results = Vec::new();

    if config.shared_connection {
        // Shared connection mode
        let client = Arc::new(Mutex::new(
            KvClient::connect(&config.server_addr)
                .await
                .map_err(|e| e.to_string())?,
        ));

        let mut handles = vec![];
        for task_id in 0..config.num_users {
            let client = Arc::clone(&client);
            let config = config.clone();
            let handle =
                tokio::spawn(async move { run_task_operations(client, config, task_id).await });
            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(task_results)) => results.extend(task_results),
                Ok(Err(e)) => return Err(format!("Task error: {}", e)),
                Err(e) => return Err(format!("Join error: {}", e)),
            }
        }
    } else {
        // Multi-connection mode
        let mut handles = vec![];
        for task_id in 0..config.num_users {
            let config = config.clone();
            let handle = tokio::spawn(async move {
                let client = KvClient::connect(&config.server_addr)
                    .await
                    .map_err(|e| e.to_string())?;
                run_task_operations(Arc::new(Mutex::new(client)), config, task_id).await
            });
            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(task_results)) => results.extend(task_results),
                Ok(Err(e)) => return Err(format!("Task error: {}", e)),
                Err(e) => return Err(format!("Join error: {}", e)),
            }
        }
    }

    Ok(results)
}

async fn run_task_operations(
    client: Arc<Mutex<KvClient>>,
    config: TestConfig,
    task_id: usize,
) -> Result<Vec<TestResult>, String> {
    let mut local_results = Vec::new();
    let mut counter_value = 0i64;

    for op_num in 0..config.operations_per_user {
        let start = Instant::now();
        let mut client_lock = client.lock().await;

        let mut operation_success = true;
        let mut operation_error = None;

        if config.full_test {
            // Comprehensive test operations
            let set_key = format!("test_key_{}_{}", task_id, op_num);
            let set_value = format!("{}", task_id);

            // SET operation
            if let Err(e) = client_lock.set(&set_key, &set_value).await {
                operation_success = false;
                operation_error = Some(format!("SET failed: {}", e));
            }

            // GET and verify
            if operation_success {
                match client_lock.get(&set_key).await {
                    Ok(Some(val)) if val == set_value => {} // OK
                    Ok(Some(val)) => {
                        operation_success = false;
                        operation_error = Some(format!(
                            "GET verification failed: expected {}, got {}",
                            set_value, val
                        ));
                    }
                    Ok(None) => {
                        operation_success = false;
                        operation_error = Some("GET returned None".to_string());
                    }
                    Err(e) => {
                        operation_success = false;
                        operation_error = Some(format!("GET failed: {}", e));
                    }
                }
            }

            // INCR and verify
            if operation_success {
                let counter_key = format!("counter_{}", task_id);
                match client_lock.incr(&counter_key).await {
                    Ok(val) => {
                        counter_value += 1;
                        if val != counter_value {
                            operation_success = false;
                            operation_error = Some(format!(
                                "INCR verification failed: expected {}, got {}",
                                counter_value, val
                            ));
                        }
                    }
                    Err(e) => {
                        operation_success = false;
                        operation_error = Some(format!("INCR failed: {}", e));
                    }
                }
            }

            // SET with TTL
            if operation_success {
                let ttl_key = format!("ttl_key_{}_{}", task_id, op_num);
                if let Err(e) = client_lock.set_with_ttl(&ttl_key, "ttl_value", "60s").await {
                    operation_success = false;
                    operation_error = Some(format!("SET with TTL failed: {}", e));
                }
            }
        }

        // Always do the batch GET (the original test)
        let batch_result = timeout(
            Duration::from_secs(3),
            client_lock.batch(&["GET loginlock-ip-123", "GET loginlock-user-abc"]),
        )
        .await;

        drop(client_lock); // Release lock

        let duration = start.elapsed();

        let final_success = match batch_result {
            Ok(Ok(_)) => operation_success,
            Ok(Err(e)) => {
                operation_success = false;
                operation_error = Some(format!("Batch GET failed: {}", e));
                false
            }
            Err(_) => {
                operation_success = false;
                operation_error = Some("Batch GET timeout".to_string());
                false
            }
        };

        local_results.push(TestResult {
            duration,
            success: final_success,
            error_type: operation_error,
        });
    }

    // Cleanup: delete test keys
    if config.full_test {
        let mut client_lock = client.lock().await;
        for op_num in 0..config.operations_per_user {
            let set_key = format!("test_key_{}_{}", task_id, op_num);
            let ttl_key = format!("ttl_key_{}_{}", task_id, op_num);
            let _ = client_lock.delete(&set_key).await; // Ignore errors
            let _ = client_lock.delete(&ttl_key).await; // Ignore errors
        }
        let counter_key = format!("counter_{}", task_id);
        let _ = client_lock.delete(&counter_key).await; // Ignore errors
    }

    Ok(local_results)
}

fn load_config(config_path: &str) -> Result<String, String> {
    let file = fs::File::open(config_path).map_err(|e| e.to_string())?;
    let reader = io::BufReader::new(file);

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.starts_with("BIND_ADDR=") {
            return Ok(line[10..].to_string());
        }
    }

    Err("BIND_ADDR not found in config".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("shrmpl-kv-loadtest")
        .arg(
            Arg::new("config")
                .help("Path to config file")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("shared")
                .long("shared")
                .help("Use shared connection mode (default: false)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("full")
                .long("full")
                .help("Run full comprehensive test (SET/GET/INCR/DELETE) instead of batch GET only")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let config_path = matches.get_one::<String>("config").unwrap();
    let server_addr = load_config(config_path)?;
    let shared_connection = matches.get_flag("shared");
    let full_test = matches.get_flag("full");

    let config = TestConfig {
        server_addr,
        num_users: 5,
        operations_per_user: 5000,
        shared_connection,
        full_test,
    };

    println!(
        "Starting load test with {} connections, {} operations each",
        config.num_users, config.operations_per_user
    );
    println!(
        "Connection mode: {}",
        if config.shared_connection {
            "shared (you can also run without --shared for multi-connection mode)"
        } else {
            "multi (you can also run with --shared for shared connection mode)"
        }
    );
    println!("Server: {}", config.server_addr);

    let test_start = Instant::now();
    let results = run_test(config).await?;
    let total_duration = test_start.elapsed();

    let total = results.len();
    let successful = results.iter().filter(|r| r.success).count();
    let errors = results.iter().filter(|r| !r.success).count();

    println!("\nLoad Test Results:");
    println!("Total Operations: {}", total);
    println!(
        "Successful: {} ({:.1}%)",
        successful,
        (successful as f64 / total as f64) * 100.0
    );
    println!(
        "Errors: {} ({:.1}%)",
        errors,
        (errors as f64 / total as f64) * 100.0
    );

    if errors > 0 {
        let mut error_counts: HashMap<String, usize> = HashMap::new();
        for result in &results {
            if let Some(ref err) = result.error_type {
                *error_counts.entry(err.clone()).or_insert(0) += 1;
            }
        }
        println!("\nError Breakdown:");
        for (err, count) in error_counts {
            println!("  {}: {}", err, count);
        }
    }

    let mut buckets = [
        (10, 0),
        (50, 0),
        (100, 0),
        (200, 0),
        (500, 0),
        (1000, 0),
        (u64::MAX, 0),
    ];
    for result in &results {
        if result.success {
            let ms = result.duration.as_millis() as u64;
            for (limit, count) in &mut buckets {
                if ms < *limit {
                    *count += 1;
                    break;
                }
            }
        }
    }

    println!("\nResponse Time Distribution (successful operations):");
    println!(
        "<10ms: {} ({:.1}%)",
        buckets[0].1,
        (buckets[0].1 as f64 / successful as f64) * 100.0
    );
    println!(
        "<50ms: {} ({:.1}%)",
        buckets[1].1,
        (buckets[1].1 as f64 / successful as f64) * 100.0
    );
    println!(
        "<100ms: {} ({:.1}%)",
        buckets[2].1,
        (buckets[2].1 as f64 / successful as f64) * 100.0
    );
    println!(
        "<200ms: {} ({:.1}%)",
        buckets[3].1,
        (buckets[3].1 as f64 / successful as f64) * 100.0
    );
    println!(
        "<500ms: {} ({:.1}%)",
        buckets[4].1,
        (buckets[4].1 as f64 / successful as f64) * 100.0
    );
    println!(
        "<1s: {} ({:.1}%)",
        buckets[5].1,
        (buckets[5].1 as f64 / successful as f64) * 100.0
    );
    println!(
        ">1s: {} ({:.1}%)",
        buckets[6].1,
        (buckets[6].1 as f64 / successful as f64) * 100.0
    );

    println!(
        "\nTotal Test Duration: {:.2}s",
        total_duration.as_secs_f64()
    );

    Ok(())
}
