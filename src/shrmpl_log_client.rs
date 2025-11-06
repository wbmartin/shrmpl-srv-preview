use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

#[derive(Clone, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn from_str(level: &str) -> Self {
        match level.to_uppercase().as_str() {
            "DEBUG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" => LogLevel::Warn,
            "ERROR" => LogLevel::Error,
            _ => LogLevel::Info, // default
        }
    }
    
    pub fn should_log(&self, message_level: &LogLevel) -> bool {
        match (self, message_level) {
            (LogLevel::Debug, _) => true,
            (LogLevel::Info, LogLevel::Info | LogLevel::Warn | LogLevel::Error) => true,
            (LogLevel::Warn, LogLevel::Warn | LogLevel::Error) => true,
            (LogLevel::Error, LogLevel::Error) => true,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct Logger {
    pub dest: String,
    pub host: String,
    pub log_level: LogLevel,
    pub log_console: bool,
    pub send_actv: bool,
    pub send_log: bool,
}

impl Logger {
    pub fn new(dest: String, host: String, log_level: LogLevel, log_console: bool, send_actv: bool, send_log: bool) -> Self {
        Self { dest, host, log_level, log_console, send_actv, send_log }
    }

    pub async fn log(&self, level: &str, code: &str, message: &str) {
        let message_level = match level {
            "DEBG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" => LogLevel::Warn,
            "ERRO" => LogLevel::Error,
            "ACTV" => LogLevel::Info, // Treat ACTV as INFO level
            _ => LogLevel::Info,
        };
        
        // Console output if enabled and level meets threshold
        if self.log_console && self.log_level.should_log(&message_level) {
            println!("{}", message);
        }
        
        // Send to SLOG if enabled and not ACTV (or ACTV is enabled)
        let should_send = self.send_log && !self.dest.is_empty() && 
                        (level != "ACTV" || self.send_actv);
        
        if should_send {
            if let Err(e) = self.send_log(level, code, message).await {
                eprintln!("Failed to send log to SLOG: {}", e);
            }
        }
    }

    pub async fn info(&self, code: &str, message: &str) {
        self.log("INFO", code, message).await;
    }

    pub async fn error(&self, code: &str, message: &str) {
        self.log("ERRO", code, message).await;
    }

    pub async fn activity(&self, code: &str, message: &str) {
        self.log("ACTV", code, message).await;
    }

    pub async fn warn(&self, code: &str, message: &str) {
        self.log("WARN", code, message).await;
    }

    pub async fn debug(&self, code: &str, message: &str) {
        self.log("DEBG", code, message).await;
    }



    // Network logging uses proper error propagation to allow graceful degradation
    // when SLOG server is unavailable - errors are logged locally but don't crash
    async fn send_log(&self, level: &str, code: &str, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Format per SLOG protocol: [LVL(4)] [HOST(32)] [CODE(4)] [LEN(4)]: [MSG]\n
        let lvl = format!("{:<4}", &level[..level.len().min(4)]);
        let host_padded = format!("{:<32}", &self.host[..self.host.len().min(32)]);
        let code_padded = format!("{:<4}", &code[..code.len().min(4)]);
        let len_str = format!("{:04}", message.len());
        let line = format!("{} {} {} {}: {}\n", lvl, host_padded, code_padded, len_str, message);
        
        let stream = timeout(Duration::from_secs(5), TcpStream::connect(&self.dest)).await??;
        let mut stream = stream;
        timeout(Duration::from_secs(5), stream.write_all(line.as_bytes())).await??;
        Ok(())
    }
}