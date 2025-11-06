use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

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

    let mut stream = match timeout(Duration::from_secs(5), TcpStream::connect(&addr)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            eprintln!("Failed to connect to {}: {}", addr, e);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("Connection timeout: Could not connect to {} within 5 seconds", addr);
            std::process::exit(1);
        }
    };
    stream.set_nodelay(true)?;

    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut stdin = BufReader::new(tokio::io::stdin());

    println!("Successfully connected to {}", addr);
    print!("?> ");
    std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable

    let mut command_buf = String::new();
    let mut response_buf = String::new();

    loop {
        tokio::select! {
            // Read from stdin
            result = stdin.read_line(&mut command_buf) => {
                match result {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let command = command_buf.trim().to_string();
                        command_buf.clear();
                        if command.is_empty() {
                            print!("?> ");
                            std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable
                            continue;
                        }
                        // Send command
                        if writer.write_all(format!("{}\n", command).as_bytes()).await.is_err() {
                            eprintln!("Failed to send command");
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Read from server
            result = reader.read_line(&mut response_buf) => {
                match result {
                    Ok(0) => {
                        println!("Connection closed by server");
                        break;
                    }
                    Ok(_) => {
                        let resp = response_buf.trim().to_string();
                        response_buf.clear();
                        // Ignore UPONG heartbeats
                        if resp == "TERM" {
                            println!("Server shutting down. Disconnecting.");
                            break;
                        } else if resp != "UPONG" {
                            println!("RECVD: {}", resp);
                            print!("?> ");
                            std::io::Write::flush(&mut std::io::stdout()).unwrap(); // stdout flush failures are unrecoverable
                        }
                    }
                    Err(_) => {
                        eprintln!("Error reading from server");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
