use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;
use crate::config::Config;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub host: String,
    pub method: String,
    pub path: String,
    pub request_headers: HashMap<String, String>,
    pub response_headers: HashMap<String, String>,
    pub status_code: u16,
    pub request_body: String,
    pub response_body: String,
    pub url_params: String,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    pub is_tunnel: bool,
    pub error: Option<String>,
}

pub struct DomainLogger {
    sender: mpsc::UnboundedSender<LogEntry>,
    config: Arc<Config>,
}

impl DomainLogger {
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let config_clone = config.clone();
        
        // 启动后台日志处理任务
        task::spawn(async move {
            while let Some(entry) = receiver.recv().await {
                Self::process_log_entry(entry, &config_clone);
            }
        });

        Arc::new(Self { sender, config })
    }

    pub fn log_request(&self, entry: LogEntry) {
        let _ = self.sender.send(entry);
    }

    fn process_log_entry(entry: LogEntry, config: &Config) {
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        use std::path::Path;
        
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        
        // 确保日志目录存在
        let log_dir = &config.logging.log_dir;
        if let Err(e) = fs::create_dir_all(log_dir) {
            eprintln!("Failed to create log directory {}: {}", log_dir, e);
            return;
        }
        
        let log_file = Path::new(log_dir).join(format!("{}_{}.log", date, entry.host));
        
        // 根据配置处理请求体
        let truncated_request_body = if config.logging.domain_logs.request_body_limit == 0 {
            String::new() // 不记录
        } else if config.logging.domain_logs.request_body_limit == -1 {
            // 完整记录
            entry.request_body.clone()
        } else if config.logging.domain_logs.request_body_limit > 0 {
            // 截断到指定长度
            let limit = config.logging.domain_logs.request_body_limit as usize;
            if entry.request_body.len() > limit {
                format!("{}... (truncated)", &entry.request_body[..limit])
            } else {
                entry.request_body.clone()
            }
        } else {
            entry.request_body.clone()
        };

        // 根据配置处理响应体
        let truncated_response_body = if config.logging.domain_logs.response_body_limit == 0 {
            String::new() // 不记录
        } else if config.logging.domain_logs.response_body_limit == -1 {
            // 完整记录
            entry.response_body.clone()
        } else if config.logging.domain_logs.response_body_limit > 0 {
            // 截断到指定长度
            let limit = config.logging.domain_logs.response_body_limit as usize;
            if entry.response_body.len() > limit {
                format!("{}... (truncated)", &entry.response_body[..limit])
            } else {
                entry.response_body.clone()
            }
        } else {
            entry.response_body.clone()
        };

        let log_line = format!(
            "[{}] {} {} {} - Status: {} - Req: {} bytes - Resp: {} bytes - Params: {} - Error: {:?}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            entry.host,
            entry.method,
            entry.path,
            entry.status_code,
            entry.request_body.len(),
            entry.response_body.len(),
            entry.url_params,
            entry.error
        );

        // 同时打印到控制台
        println!("{}", log_line);
        
        // 写入到域名对应的日志文件
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
        {
            let _ = writeln!(file, "{}", log_line);
            
            // 写入详细信息
            let _ = writeln!(file, "  Request Headers: {:?}", entry.request_headers);
            let _ = writeln!(file, "  Response Headers: {:?}", entry.response_headers);
            if !entry.request_body.is_empty() {
                let _ = writeln!(file, "  Request Body: {}", truncated_request_body);
            }
            if !entry.response_body.is_empty() {
                let _ = writeln!(file, "  Response Body: {}", truncated_response_body);
            }
            let _ = writeln!(file, "---");
        } else {
            eprintln!("Failed to write log to file: {}", log_file.display());
        }
    }

    pub fn create_log_entry(
        host: String,
        method: String,
        path: String,
        request_headers: HashMap<String, String>,
        response_headers: HashMap<String, String>,
        status_code: u16,
        request_body: String,
        response_body: String,
        url_params: String,
        bytes_sent: usize,
        bytes_received: usize,
        is_tunnel: bool,
        error: Option<String>,
    ) -> LogEntry {
        LogEntry {
            host,
            method,
            path,
            request_headers,
            response_headers,
            status_code,
            request_body,
            response_body,
            url_params,
            bytes_sent,
            bytes_received,
            is_tunnel,
            error,
        }
    }

    pub fn create_tunnel_log_entry(
        host: String,
        bytes_sent: usize,
        bytes_received: usize,
        error: Option<String>,
    ) -> LogEntry {
        LogEntry {
            host,
            method: "CONNECT".to_string(),
            path: "TUNNEL".to_string(),
            request_headers: HashMap::new(),
            response_headers: HashMap::new(),
            status_code: 200,
            request_body: String::new(),
            response_body: String::new(),
            url_params: String::new(),
            bytes_sent,
            bytes_received,
            is_tunnel: true,
            error,
        }
    }
}

// 移除Default实现，因为需要Config参数