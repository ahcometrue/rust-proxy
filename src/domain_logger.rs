use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;
use crate::config::Config;
use std::io::Write;

/// 日志条目结构体
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// 主机名
    pub host: String,
    /// HTTP方法
    pub method: String,
    /// 请求路径
    pub path: String,
    /// 请求头
    pub request_headers: HashMap<String, String>,
    /// 响应头
    pub response_headers: HashMap<String, String>,
    /// 状态码
    pub status_code: u16,
    /// 请求体
    pub request_body: String,
    /// 响应体
    pub response_body: String,
    /// URL参数
    pub url_params: String,
    /// 错误信息
    pub error: Option<String>,
    /// 处理耗时（毫秒）
    pub duration_ms: u128,
}

/// 域名日志记录器
pub struct DomainLogger {
    /// 日志发送通道
    sender: mpsc::UnboundedSender<LogEntry>,
}

impl DomainLogger {
    /// 创建新的域名日志记录器
    /// 
    /// # 参数
    /// * `config` - 配置信息
    /// 
    /// # 返回值
    /// 返回Arc包装的DomainLogger实例
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let config_clone = config.clone();
        
        // 启动后台日志处理任务
        task::spawn(async move {
            while let Some(entry) = receiver.recv().await {
                Self::process_log_entry(entry, &config_clone);
            }
        });

        Arc::new(Self { sender })
    }

    /// 记录请求日志
    /// 
    /// # 参数
    /// * `entry` - 日志条目
    pub fn log_request(&self, entry: LogEntry) {
        // 忽略发送错误，因为这通常意味着接收端已关闭
        let _ = self.sender.send(entry);
    }

    /// 处理日志条目
    /// 
    /// # 参数
    /// * `entry` - 日志条目
    /// * `config` - 配置信息
    fn process_log_entry(entry: LogEntry, config: &Config) {
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        use std::path::Path;
        use chrono::Local;
        
        let date = Local::now().format("%Y-%m-%d").to_string();
        
        // 确保日志目录存在
        let log_dir = &config.logging.log_dir;
        if let Err(e) = fs::create_dir_all(log_dir) {
            eprintln!("Failed to create log directory {log_dir}: {e}");
            return;
        }
        
        let log_file = Path::new(log_dir).join(format!("{}_{}.log", date, entry.host));
        
        // 根据配置处理请求体
        let truncated_request_body = Self::process_body_content_helper(
            &entry.request_body, 
            config.logging.domain_logs.request_body_limit
        );

        // 根据配置处理响应体
        let truncated_response_body = Self::process_body_content_helper(
            &entry.response_body, 
            config.logging.domain_logs.response_body_limit
        );

        let log_line = format!(
            "[{}] {} {} {} - Status: {} - Duration: {}ms - Req: {} bytes - Resp: {} bytes - Params: {} - Error: {:?}",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            entry.host,
            entry.method,
            entry.path,
            entry.status_code,
            entry.duration_ms,
            entry.request_body.len(),
            entry.response_body.len(),
            entry.url_params,
            entry.error
        );

        // 同时打印到控制台
        println!("{log_line}");
        
        // 写入到域名对应的日志文件
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
        {
            let _ = writeln!(file, "{log_line}");
            
            // 写入详细信息
            let _ = writeln!(file, "  Request Headers: {:?}", entry.request_headers);
            let _ = writeln!(file, "  Response Headers: {:?}", entry.response_headers);
            
            // 根据内容是否为空决定是否写入
            Self::write_body_content_helper(&mut file, "Request Body", &truncated_request_body);
            Self::write_body_content_helper(&mut file, "Response Body", &truncated_response_body);
            
            let _ = writeln!(file, "---");
        } else {
            eprintln!("Failed to write log to file: {}", log_file.display());
        }
    }

    /// 处理请求体/响应体内容辅助函数
    /// 
    /// # 参数
    /// * `body` - 原始内容
    /// * `limit` - 限制大小
    /// 
    /// # 返回值
    /// 处理后的内容
    fn process_body_content_helper(body: &str, limit: i64) -> String {
        match limit {
            0 => String::new(), // 不记录
            -1 => body.to_string(), // 完整记录
            limit if limit > 0 => {
                // 截断到指定长度
                let limit = limit as usize;
                if body.len() > limit {
                    format!("{}... (truncated)", &body[..limit])
                } else {
                    body.to_string()
                }
            },
            _ => body.to_string(), // 默认情况，完整记录
        }
    }

    /// 写入请求体/响应体内容到文件辅助函数
    /// 
    /// # 参数
    /// * `file` - 文件句柄
    /// * `label` - 标签（Request Body或Response Body）
    /// * `content` - 内容
    fn write_body_content_helper(file: &mut std::fs::File, label: &str, content: &str) {
        match content.is_empty() {
            true => (), // 内容为空时不写入
            false => {
                let _ = writeln!(file, "  {label}: {content}");
            }
        }
    }

    /// 创建日志条目
    /// 
    /// # 参数
    /// * `host` - 主机名
    /// * `method` - HTTP方法
    /// * `path` - 请求路径
    /// * `request_headers` - 请求头
    /// * `response_headers` - 响应头
    /// * `status_code` - 状态码
    /// * `request_body` - 请求体
    /// * `response_body` - 响应体
    /// * `url_params` - URL参数
    /// * `duration_ms` - 处理耗时（毫秒）
    /// * `error` - 错误信息
    /// 
    /// # 返回值
    /// 返回构建的LogEntry实例
    #[allow(clippy::too_many_arguments)]
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
        duration_ms: u128,
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
            error,
            duration_ms,
        }
    }

    /// 创建隧道模式日志条目
    /// 
    /// # 参数
    /// * `host` - 主机名
    /// * `duration_ms` - 处理耗时（毫秒）
    /// * `error` - 错误信息
    /// 
    /// # 返回值
    /// 返回构建的LogEntry实例
    pub fn create_tunnel_log_entry(
        host: String,
        duration_ms: u128,
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
            error,
            duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_config(log_dir: &str) -> Config {
        Config {
            proxy: crate::config::ProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 8888,
            },
            target: crate::config::TargetConfig {
                domains: vec!["*".to_string()],
                ports: vec![0],
            },
            certificates: crate::config::CertificatesConfig {
                ca_cert: "certs/ca.crt".to_string(),
                ca_key: "certs/ca.key".to_string(),
                auto_install: true,
                auto_uninstall: true,
                name: "study-proxy".to_string(),
            },
            system_proxy: crate::config::SystemProxyConfig {
                enabled: true,
                auto_configure: true,
            },
            logging: crate::config::LoggingConfig {
                level: "debug".to_string(),
                output: "file".to_string(),
                log_dir: log_dir.to_string(),
                program_log: "proxy.log".to_string(),
                domain_logs: crate::config::DomainLogsConfig {
                    enabled: true,
                    format: "domain_{domain}_{date}.log".to_string(),
                    request_body_limit: 1024,
                    response_body_limit: 1024,
                },
            },
        }
    }

    #[tokio::test]
    async fn test_create_domain_logger() {
        let temp_dir = TempDir::new().unwrap();
        let log_dir = temp_dir.path().to_str().unwrap().to_string();
        
        let config = Arc::new(create_test_config(&log_dir));
        let logger = DomainLogger::new(config);
        
        assert!(!logger.sender.is_closed());
    }

    #[tokio::test]
    async fn test_create_log_entry() {
        let temp_dir = TempDir::new().unwrap();
        let log_dir = temp_dir.path().to_str().unwrap().to_string();
        
        let config = Arc::new(create_test_config(&log_dir));
        let logger = DomainLogger::new(config);
        
        let mut request_headers = HashMap::new();
        request_headers.insert("User-Agent".to_string(), "test-agent".to_string());
        request_headers.insert("Content-Type".to_string(), "application/json".to_string());
        
        let mut response_headers = HashMap::new();
        response_headers.insert("Content-Type".to_string(), "application/json".to_string());
        response_headers.insert("Server".to_string(), "test-server".to_string());
        
        let log_entry = DomainLogger::create_log_entry(
            "example.com".to_string(),
            "GET".to_string(),
            "/test".to_string(),
            request_headers.clone(),
            response_headers.clone(),
            200,
            "test request body".to_string(),
            "test response body".to_string(),
            "param1=value1&param2=value2".to_string(),
            150, // duration_ms
            None,
        );
        
        logger.log_request(log_entry.clone());
        
        // 验证日志条目内容
        assert_eq!(log_entry.host, "example.com");
        assert_eq!(log_entry.method, "GET");
        assert_eq!(log_entry.path, "/test");
        assert_eq!(log_entry.status_code, 200);
        assert_eq!(log_entry.request_body, "test request body");
        assert_eq!(log_entry.response_body, "test response body");
        assert_eq!(log_entry.url_params, "param1=value1&param2=value2");
        assert_eq!(log_entry.error, None);
        assert_eq!(log_entry.request_headers, request_headers);
        assert_eq!(log_entry.response_headers, response_headers);
        assert_eq!(log_entry.duration_ms, 150);
    }

    #[test]
    fn test_create_tunnel_log_entry() {
        let log_entry = DomainLogger::create_tunnel_log_entry(
            "example.com".to_string(),
            200, // duration_ms
            Some("test error".to_string()),
        );
        
        assert_eq!(log_entry.host, "example.com");
        assert_eq!(log_entry.method, "CONNECT");
        assert_eq!(log_entry.path, "TUNNEL");
        assert_eq!(log_entry.status_code, 200);
        assert_eq!(log_entry.request_body, "");
        assert_eq!(log_entry.response_body, "");
        assert_eq!(log_entry.url_params, "");
        assert_eq!(log_entry.error, Some("test error".to_string()));
        assert!(log_entry.request_headers.is_empty());
        assert!(log_entry.response_headers.is_empty());
        assert_eq!(log_entry.duration_ms, 200);
    }

    #[test]
    fn test_process_body_content_helper() {
        // 测试不记录情况 (limit = 0)
        assert_eq!(DomainLogger::process_body_content_helper("test body", 0), "");
        
        // 测试完整记录情况 (limit = -1)
        assert_eq!(DomainLogger::process_body_content_helper("test body", -1), "test body");
        
        // 测试正常截断情况
        assert_eq!(
            DomainLogger::process_body_content_helper("this is a long body content", 10),
            "this is a ... (truncated)"
        );
        
        // 测试不需要截断的情况
        assert_eq!(
            DomainLogger::process_body_content_helper("short", 10),
            "short"
        );
    }

    #[test]
    fn test_write_body_content_helper() {
        let temp_dir = TempDir::new().unwrap();
        let log_file = temp_dir.path().join("test.log");
        
        // 测试写入非空内容
        {
            let mut file = std::fs::File::create(&log_file).unwrap();
            DomainLogger::write_body_content_helper(&mut file, "Test Label", "test content");
        }
        
        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("Test Label: test content"));
        
        // 测试写入空内容（应该不写入任何内容）
        std::fs::remove_file(&log_file).unwrap(); // 先清空文件
        {
            let mut file = std::fs::File::create(&log_file).unwrap();
            DomainLogger::write_body_content_helper(&mut file, "Test Label", "");
        }
        
        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.is_empty());
    }
}