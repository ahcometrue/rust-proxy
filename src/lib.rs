pub mod config;
pub mod cert;
pub mod domain_logger;

// 公共导出
pub use config::Config;
pub use cert::CertManager;
pub use domain_logger::{DomainLogger, LogEntry};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        // 创建一个临时的配置文件用于测试
        let config_content = r#"{
            "proxy": {
                "host": "127.0.0.1",
                "port": 8888
            },
            "target": {
                "domains": ["*"],
                "ports": ["*"]
            },
            "certificates": {
                "ca_cert": "certs/ca.crt",
                "ca_key": "certs/ca.key"
            },
            "logging": {
                "level": "debug",
                "output": "file",
                "log_dir": "logs",
                "program_log": "proxy.log",
                "domain_logs": {
                    "enabled": true,
                    "format": "domain_{domain}_{date}.log",
                    "request_body_limit": 1024,
                    "response_body_limit": 1024
                }
            }
        }"#;
        
        // 将配置写入临时文件
        std::fs::write("test_config.json", config_content).unwrap();
        
        // 加载配置
        let config = Config::from_file("test_config.json").expect("Failed to load config");
        
        // 验证配置加载正确
        assert_eq!(config.proxy.host, "127.0.0.1");
        assert_eq!(config.proxy.port, 8888);
        assert_eq!(config.target.domains, vec!["*"]);
        assert_eq!(config.certificates.ca_cert, "certs/ca.crt");
        assert_eq!(config.certificates.ca_key, "certs/ca.key");
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.log_dir, "logs");
        assert!(config.logging.domain_logs.enabled);
        assert_eq!(config.logging.domain_logs.request_body_limit, 1024);
        assert_eq!(config.logging.domain_logs.response_body_limit, 1024);
        
        // 清理临时文件
        std::fs::remove_file("test_config.json").unwrap();
    }
    
    #[test]
    fn test_should_intercept() {
        let config_content = r#"{
            "proxy": {
                "host": "127.0.0.1",
                "port": 8888
            },
            "target": {
                "domains": ["example.com", "test.com"],
                "ports": [80, 443]
            },
            "certificates": {
                "ca_cert": "certs/ca.crt",
                "ca_key": "certs/ca.key"
            },
            "logging": {
                "level": "debug",
                "output": "file",
                "log_dir": "logs",
                "program_log": "proxy.log",
                "domain_logs": {
                    "enabled": true,
                    "format": "domain_{domain}_{date}.log",
                    "request_body_limit": 1024,
                    "response_body_limit": 1024
                }
            }
        }"#;
        
        // 将配置写入临时文件
        std::fs::write("test_config2.json", config_content).unwrap();
        
        // 加载配置
        let config = Config::from_file("test_config2.json").expect("Failed to load config");
        
        // 测试精确匹配
        assert!(config.should_intercept("example.com", 80));
        assert!(config.should_intercept("test.com", 443));
        
        // 测试部分匹配
        assert!(config.should_intercept("api.example.com", 80));
        assert!(config.should_intercept("sub.test.com", 443));
        
        // 测试不匹配的情况
        assert!(!config.should_intercept("other.com", 80));
        assert!(!config.should_intercept("example.com", 8080));
        
        // 清理临时文件
        std::fs::remove_file("test_config2.json").unwrap();
    }
    
    #[test]
    fn test_wildcard_intercept() {
        let config_content = r#"{
            "proxy": {
                "host": "127.0.0.1",
                "port": 8888
            },
            "target": {
                "domains": ["*"],
                "ports": ["*"]
            },
            "certificates": {
                "ca_cert": "certs/ca.crt",
                "ca_key": "certs/ca.key"
            },
            "logging": {
                "level": "debug",
                "output": "file",
                "log_dir": "logs",
                "program_log": "proxy.log",
                "domain_logs": {
                    "enabled": true,
                    "format": "domain_{domain}_{date}.log",
                    "request_body_limit": 1024,
                    "response_body_limit": 1024
                }
            }
        }"#;
        
        // 将配置写入临时文件
        std::fs::write("test_config3.json", config_content).unwrap();
        
        // 加载配置
        let config = Config::from_file("test_config3.json").expect("Failed to load config");
        
        // 测试通配符匹配
        assert!(config.should_intercept("example.com", 80));
        assert!(config.should_intercept("test.com", 443));
        assert!(config.should_intercept("any.domain.com", 8080));
        
        // 清理临时文件
        std::fs::remove_file("test_config3.json").unwrap();
    }
}