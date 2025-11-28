use tokio::io::{AsyncBufReadExt, BufReader};
use shrmpl::shrmpl_kv_client::KvClient;

// Client application uses proper error propagation to provide user-friendly error messages
// and allow for graceful error handling (e.g., connection timeouts, network errors)
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <ip> <port>", args[0]);
        eprintln!("Example: {} 127.0.0.1 7171", args[0]);
        std::process::exit(1);
    }
    let ip = &args[1];
    let port = &args[2];
    let addr = format!("{}:{}", ip, port);

    let mut client = match KvClient::connect(&addr).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    println!("Successfully connected to {}", addr);
    print!("?> ");
    std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut command_buf = String::new();

    loop {
        command_buf.clear();
        match stdin.read_line(&mut command_buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let command = command_buf.trim().to_string();
                if command.is_empty() {
                    print!("?> ");
                    std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable
                    continue;
                }

                let parts: Vec<&str> = command.split_whitespace().collect();
                if parts.is_empty() {
                    print!("?> ");
                    std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable
                    continue;
                }

                let cmd = parts[0].to_uppercase();
                match cmd.as_str() {
                    "GET" => {
                        if parts.len() != 2 {
                            println!("ERROR invalid arguments");
                        } else {
                            match client.get(parts[1]).await {
                                Ok(Some(value)) => println!("{}", value),
                                Ok(None) => println!("ERROR key not found"),
                                Err(e) => println!("ERROR: {}", e),
                            }
                        }
                    }
                    "SET" => {
                        if parts.len() < 3 || parts.len() > 4 {
                            println!("ERROR invalid arguments");
                        } else if parts.len() == 3 {
                            match client.set(parts[1], parts[2]).await {
                                Ok(_) => println!("OK"),
                                Err(e) => println!("ERROR: {}", e),
                            }
                        } else {
                            match client.set_with_ttl(parts[1], parts[2], parts[3]).await {
                                Ok(_) => println!("OK"),
                                Err(e) => println!("ERROR: {}", e),
                            }
                        }
                    }
                    "INCR" => {
                        if parts.len() < 2 || parts.len() > 3 {
                            println!("ERROR invalid arguments");
                        } else if parts.len() == 2 {
                            match client.incr(parts[1]).await {
                                Ok(value) => println!("{}", value),
                                Err(e) => println!("ERROR: {}", e),
                            }
                        } else {
                            match client.incr_with_ttl(parts[1], parts[2]).await {
                                Ok(value) => println!("{}", value),
                                Err(e) => println!("ERROR: {}", e),
                            }
                        }
                    }
                    "DEL" => {
                        if parts.len() != 2 {
                            println!("ERROR invalid arguments");
                        } else {
                            match client.delete(parts[1]).await {
                                Ok(deleted) => {
                                    if deleted {
                                        println!("OK")
                                    } else {
                                        println!("ERROR key not found")
                                    }
                                }
                                Err(e) => println!("ERROR: {}", e),
                            }
                        }
                    }
                    "PING" => {
                        match client.ping().await {
                            Ok(_) => println!("PONG"),
                            Err(e) => println!("ERROR: {}", e),
                        }
                    }
                    "LIST" => {
                        if parts.len() != 1 {
                            println!("ERROR invalid arguments");
                        } else {
                            match client.list().await {
                                Ok(items) => {
                                    if items.is_empty() {
                                        println("(no keys)");
                                    } else {
                                        for (key, value, expiration) in items {
                                            match expiration {
                                                Some(timestamp) => {
                                                    let datetime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp);
                                                    println!("{} = {} (expires: {:?})", key, value, datetime);
                                                }
                                                None => {
                                                    println!("{} = {} (no expiration)", key, value);
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => println!("ERROR: {}", e),
                            }
                        }
                    }
                    _ => {
                        println!("ERROR unknown command");
                    }
                }

                print!("?> ");
                std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable
            }
            Err(_) => break,
        }
    }

    Ok(())
}