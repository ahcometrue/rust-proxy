use anyhow::Result;
use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::collections::HashMap;
use crate::config::LoggingConfig;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub domain: String,
    pub method: String,
    pub url: String,
    pub request_info: RequestInfo,
    pub response_info: ResponseInfo,
}

#[derive(Debug, Clone)]
pub struct RequestInfo {
    pub url: String,
    pub params: HashMap<String, String>,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResponseInfo {
    pub url: String,
    pub params: HashMap<String, String>,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub status: Option<u16>,
    pub body: Option<String>,
    pub size: usize,
}

impl LogEntry {
    pub fn new(domain: &str, method: &str, url: &str) -> Self {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let url_params = Self::parse_url_params(url);
        
        Self {
            timestamp,
            domain: domain.to_string(),
            method: method.to_string(),
            url: url.to_string(),
            request_info: RequestInfo {
                url: url.to_string(),
                params: url_params.clone(),
                method: method.to_string(),
                headers: HashMap::new(),
                body: None,
            },
            response_info: ResponseInfo {
                url: url.to_string(),
                params: url_params,
                method: method.to_string(),
                headers: HashMap::new(),
                status: None,
                body: None,
                size: 0,
            },
        }
    }

    fn parse_url_params(url: &str) -> HashMap<String, String> {
        let mut params = HashMap::new();
        if let Some(query_start) = url.find('?') {
            let query = &url[query_start + 1..];
            for pair in query.split('&') {
                if let Some(eq_pos) = pair.find('=') {
                    let key = pair[..eq_pos].to_string();
                    let value = pair[eq_pos + 1..].to_string();
                    params.insert(key, value);
                }
            }
        }
        params
    }

    pub fn to_json(&self) -> String {
        format!(
            r#"{{
  "timestamp": "{}",
  "domain": "{}",
  "method": "{}",
  "url": "{}",
  "request": {{
    "url": "{}",
    "params": {:?},
    "method": "{}",
    "headers": {:?},
    "body": {}
  }},
  "response": {{
    "url": "{}",
    "params": {:?},
    "method": "{}",
    "headers": {:?},
    "status": {},
    "body": {},
    "size": {}
  }}
}}"#,
            self.timestamp,
            self.domain,
            self.method,
            self.url,
            self.request_info.url,
            self.request_info.params,
            self.request_info.method,
            self.request_info.headers,
            self.request_info.body.as_ref().map_or("null".to_string(), |b| format!("\"{}\"", b.replace('"', "\\\""))),
            self.response_info.url,
            self.response_info.params,
            self.response_info.method,
            self.response_info.headers,
            self.response_info.status.map_or("null".to_string(), |s| s.to_string()),
            self.response_info.body.as_ref().map_or("null".to_string(), |b| format!("\"{}\"", b.replace('"', "\\\""))),
            self.response_info.size
        )
    }
}

#[derive(Clone)]
pub struct Logger {
    log_dir: String,
    _program_log: String,  // 添加下划线前缀表示未使用但保留字段
}

impl Logger {
    pub fn new(config: &LoggingConfig) -> Result<Self> {
        fs::create_dir_all(&config.log_dir)?;
        Ok(Self {
            log_dir: config.log_dir.clone(),
            _program_log: config.program_log.clone(),  // 添加下划线前缀表示未使用但保留字段
        })
    }

    pub fn log_request(&self, entry: LogEntry) -> Result<()> {
        if true {  // 假设域名日志始终启用
            log::debug!("Domain logs disabled");
            return Ok(());
        }

        let _date = Local::now().format("%Y%m%d").to_string();  // 添加下划线前缀表示未使用但保留变量
        // 提取主机名部分，移除协议、端口和路径
        let host_only = {
            // 移除协议部分
            let without_protocol = if let Some(pos) = entry.domain.find("://") {
                &entry.domain[pos + 3..]
            } else {
                &entry.domain
            };
            
            // 移除路径部分
            let without_path = without_protocol.split('/').next().unwrap_or(without_protocol);
            
            // 移除端口部分
            let without_port = without_path.split(':').next().unwrap_or(without_path);
            
            without_port
        };
            
        // 替换域名中的特殊字符，确保文件名有效
        let safe_domain = host_only
            .replace(['.', ':', '/', '\\', '?', '&', '=', '%'], "_");
        
        // 使用默认格式
        let filename = format!("domain_{safe_domain}.log");
            
        let filepath = Path::new(&self.log_dir).join(&filename);
        
        log::debug!("Attempting to create domain log file: {filepath:?}");
        log::debug!("Domain: {}", entry.domain);
        log::debug!("Host only: {host_only}");
        log::debug!("Safe domain: {safe_domain}");
        log::debug!("Filename: {filename}");
        
        let log_entry = format!("{}, {} {safe_domain}.log", Local::now().format("%Y-%m-%d %H:%M:%S"), entry.to_json());
        
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filepath) {
            Ok(mut file) => {
                match writeln!(file, "{log_entry}") {
                    Ok(_) => log::debug!("Successfully wrote to domain log: {filepath:?}"),
                    Err(e) => log::error!("Failed to write to domain log {filepath:?}: {e}"),
                }
            }
            Err(e) => log::error!("Failed to open domain log file {filepath:?}: {e}"),
        }
        
        Ok(())
    }
}