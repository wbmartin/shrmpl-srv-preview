use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

pub struct KvClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
}

impl KvClient {
    pub async fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let stream = match timeout(Duration::from_secs(5), TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                return Err(format!("Failed to connect to {}: {}", addr, e).into());
            }
            Err(_) => {
                return Err(format!("Connection timeout: Could not connect to {} within 5 seconds", addr).into());
            }
        };
        
        stream.set_nodelay(true)?;
        let (reader, writer) = stream.into_split();
        
        Ok(KvClient {
            reader: BufReader::new(reader),
            writer,
        })
    }

    async fn send_command(&mut self, cmd: &str) -> Result<String, Box<dyn std::error::Error>> {
        if self.writer.write_all(format!("{}\n", cmd).as_bytes()).await.is_err() {
            return Err("Failed to send command".into());
        }

        let mut response = String::new();
        loop {
            response.clear();
            match self.reader.read_line(&mut response).await {
                Ok(0) => return Err("Connection closed by server".into()),
                Ok(_) => {
                    let resp = response.trim().to_string();
                    // Ignore UPONG heartbeats, return everything else
                    if resp == "UPONG" {
                        continue;
                    } else if resp == "TERM" {
                        return Err("Server shutting down".into());
                    } else {
                        return Ok(resp);
                    }
                }
                Err(_) => return Err("Error reading from server".into()),
            }
        }
    }

    pub async fn get(&mut self, key: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if key.len() > 100 {
            return Err("Key length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("GET {}", key)).await?;
        
        if response.starts_with("ERROR") {
            if response.contains("key not found") {
                Ok(None)
            } else {
                Err(response.into())
            }
        } else {
            Ok(Some(response))
        }
    }

    pub async fn set(&mut self, key: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
        if key.len() > 100 || value.len() > 100 {
            return Err("Key or value length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("SET {} {}", key, value)).await?;
        
        if response == "OK" {
            Ok(())
        } else {
            Err(response.into())
        }
    }

    pub async fn set_with_ttl(&mut self, key: &str, value: &str, ttl: &str) -> Result<(), Box<dyn std::error::Error>> {
        if key.len() > 100 || value.len() > 100 {
            return Err("Key or value length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("SET {} {} {}", key, value, ttl)).await?;
        
        if response == "OK" {
            Ok(())
        } else {
            Err(response.into())
        }
    }

    pub async fn incr(&mut self, key: &str) -> Result<i64, Box<dyn std::error::Error>> {
        if key.len() > 100 {
            return Err("Key length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("INCR {}", key)).await?;
        
        if response.starts_with("ERROR") {
            Err(response.into())
        } else {
            response.parse::<i64>().map_err(|e| e.into())
        }
    }

    pub async fn incr_with_ttl(&mut self, key: &str, ttl: &str) -> Result<i64, Box<dyn std::error::Error>> {
        if key.len() > 100 {
            return Err("Key length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("INCR {} {}", key, ttl)).await?;
        
        if response.starts_with("ERROR") {
            Err(response.into())
        } else {
            response.parse::<i64>().map_err(|e| e.into())
        }
    }

    pub async fn delete(&mut self, key: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if key.len() > 100 {
            return Err("Key length exceeds 100 characters".into());
        }

        let response = self.send_command(&format!("DEL {}", key)).await?;
        
        if response == "OK" {
            Ok(true)
        } else if response.contains("key not found") {
            Ok(false)
        } else {
            Err(response.into())
        }
    }

    pub async fn ping(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let response = self.send_command("PING").await?;
        
        if response == "PONG" {
            Ok(())
        } else {
            Err(response.into())
        }
    }

    pub async fn list(&mut self) -> Result<Vec<(String, String, Option<u64>)>, Box<dyn std::error::Error>> {
        let response = self.send_command("LIST").await?;
        
        if response.starts_with("ERROR") {
            Err(response.into())
        } else {
            let mut result = Vec::new();
            if response.trim().is_empty() {
                return Ok(result);
            }
            
            for line in response.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                
                let parts: Vec<&str> = line.splitn(3, '=').collect();
                if parts.len() != 3 {
                    continue;
                }
                
                let key = parts[0].to_string();
                let value_and_expiration: Vec<&str> = parts[2].split(',').collect();
                if value_and_expiration.len() != 2 {
                    continue;
                }
                
                let value = value_and_expiration[0].to_string();
                let expiration = if value_and_expiration[1] == "no-expiration" {
                    None
                } else {
                    value_and_expiration[1].parse::<u64>().ok()
                };
                
                result.push((key, value, expiration));
            }
            
            Ok(result)
        }
    }
}