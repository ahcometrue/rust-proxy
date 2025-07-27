use serde::{Deserialize, Serialize};
use std::fs;
use serde::de::{self, Visitor};
use std::fmt;

/// 代理服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 监听主机地址
    pub host: String,
    /// 监听端口
    pub port: u16,
}

/// 目标配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    /// 拦截的域名列表
    pub domains: Vec<String>,
    /// 拦截的端口列表
    #[serde(deserialize_with = "deserialize_ports")]
    pub ports: Vec<u16>,
}

/// 证书配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificatesConfig {
    /// CA证书文件路径
    pub ca_cert: String,
    /// CA私钥文件路径
    pub ca_key: String,
}

/// 域名日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainLogsConfig {
    /// 是否启用域名日志
    pub enabled: bool,
    /// 日志文件名格式
    pub format: String,
    /// 请求体大小限制
    #[serde(default = "default_request_body_limit")]
    pub request_body_limit: i64,
    /// 响应体大小限制
    #[serde(default = "default_response_body_limit")]
    pub response_body_limit: i64,
}

/// 默认请求体大小限制
fn default_request_body_limit() -> i64 {
    1024
}

/// 默认响应体大小限制
fn default_response_body_limit() -> i64 {
    1024
}

/// 系统代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemProxyConfig {
    /// 是否启用系统代理
    pub enabled: bool,
    /// 是否自动配置系统代理
    pub auto_configure: bool,
}

impl Default for SystemProxyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_configure: true,
        }
    }
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// 日志级别
    pub level: String,
    /// 日志输出方式
    pub output: String,
    /// 日志目录
    pub log_dir: String,
    /// 程序日志文件名
    pub program_log: String,
    /// 域名日志配置
    pub domain_logs: DomainLogsConfig,
}

/// 主配置结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 代理配置
    pub proxy: ProxyConfig,
    /// 系统代理配置
    #[serde(default)]
    pub system_proxy: SystemProxyConfig,
    /// 目标配置
    pub target: TargetConfig,
    /// 证书配置
    pub certificates: CertificatesConfig,
    /// 日志配置
    pub logging: LoggingConfig,
}

impl Config {
    /// 从文件加载配置
    /// 
    /// # 参数
    /// * `path` - 配置文件路径
    /// 
    /// # 返回值
    /// 返回Result包装的Config实例，如果过程中出现错误则返回错误信息
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// 判断是否应该拦截指定域名和端口的请求
    /// 
    /// # 参数
    /// * `domain` - 域名
    /// * `port` - 端口
    /// 
    /// # 返回值
    /// 如果应该拦截返回true，否则返回false
    pub fn should_intercept(&self, domain: &str, port: u16) -> bool {
        let domain_match = self.target.domains.iter().any(|d| {
            match d.as_str() {
                "*" => true,  // 通配符匹配所有域名
                d_str => domain.contains(d_str),  // 部分匹配
            }
        });
        
        let port_match = self.target.ports.iter().any(|&p| {
            match p {
                0 => true,   // 0表示通配符，匹配所有端口
                p_val => p_val == port,  // 精确匹配
            }
        });
        
        domain_match && port_match
    }
}

/// 端口反序列化函数
fn deserialize_ports<'de, D>(deserializer: D) -> Result<Vec<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct PortsVisitor;

    impl<'de> Visitor<'de> for PortsVisitor {
        type Value = Vec<u16>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a vector of numbers or a wildcard string")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut ports = Vec::new();
            while let Some(value) = seq.next_element::<serde_json::Value>()? {
                match value {
                    serde_json::Value::Number(n) => {
                        if let Some(port) = n.as_u64() {
                            ports.push(port as u16);
                        }
                    }
                    serde_json::Value::String(s) => {
                        match s.as_str() {
                            "*" => ports.push(0), // 0 表示通配符
                            s_str => {
                                if let Ok(port) = s_str.parse::<u16>() {
                                    ports.push(port);
                                }
                            }
                        }
                    }
                    _ => return Err(de::Error::custom("invalid port format")),
                }
            }
            Ok(ports)
        }
    }

    deserializer.deserialize_seq(PortsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_file() {
        let config_content = r#"{
            "proxy": {
                "host": "127.0.0.1",
                "port": 8888
            },
            "target": {
                "domains": ["example.com"],
                "ports": [80, 443]
            },
            "certificates": {
                "ca_cert": "certs/ca.crt",
                "ca_key": "certs/ca.key"
            },
            "system_proxy": {
                "enabled": true,
                "auto_configure": true
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
        
        std::fs::write("test_config_file.json", config_content).unwrap();
        let config = Config::from_file("test_config_file.json").expect("Failed to load config");
        
        assert_eq!(config.proxy.host, "127.0.0.1");
        assert_eq!(config.proxy.port, 8888);
        assert_eq!(config.system_proxy.enabled, true);
        assert_eq!(config.system_proxy.auto_configure, true);
        assert_eq!(config.target.domains, vec!["example.com"]);
        assert_eq!(config.target.ports, vec![80, 443]);
        assert_eq!(config.certificates.ca_cert, "certs/ca.crt");
        assert_eq!(config.certificates.ca_key, "certs/ca.key");
        assert_eq!(config.logging.level, "debug");
        assert!(config.logging.domain_logs.enabled);
        assert_eq!(config.logging.domain_logs.request_body_limit, 1024);
        
        std::fs::remove_file("test_config_file.json").unwrap();
    }
    
    #[test]
    fn test_should_intercept_exact_match() {
        let config = Config {
            proxy: ProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 8888,
            },
            target: TargetConfig {
                domains: vec!["example.com".to_string()],
                ports: vec![80, 443],
            },
            certificates: CertificatesConfig {
                ca_cert: "certs/ca.crt".to_string(),
                ca_key: "certs/ca.key".to_string(),
            },
            system_proxy: SystemProxyConfig::default(),
            logging: LoggingConfig {
                level: "debug".to_string(),
                output: "file".to_string(),
                log_dir: "logs".to_string(),
                program_log: "proxy.log".to_string(),
                domain_logs: DomainLogsConfig {
                    enabled: true,
                    format: "domain_{domain}_{date}.log".to_string(),
                    request_body_limit: 1024,
                    response_body_limit: 1024,
                },
            },
        };
        
        // 测试精确匹配
        assert!(config.should_intercept("example.com", 80));
        assert!(config.should_intercept("example.com", 443));
        
        // 测试不匹配的情况
        assert!(!config.should_intercept("other.com", 80));
        assert!(!config.should_intercept("example.com", 8080));
    }
    
    #[test]
    fn test_should_intercept_wildcard_domains() {
        let config = Config {
            proxy: ProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 8888,
            },
            target: TargetConfig {
                domains: vec!["*".to_string()],
                ports: vec![80, 443],
            },
            certificates: CertificatesConfig {
                ca_cert: "certs/ca.crt".to_string(),
                ca_key: "certs/ca.key".to_string(),
            },
            logging: LoggingConfig {
                level: "debug".to_string(),
                output: "file".to_string(),
                log_dir: "logs".to_string(),
                program_log: "proxy.log".to_string(),
                domain_logs: DomainLogsConfig {
                    enabled: true,
                    format: "domain_{domain}_{date}.log".to_string(),
                    request_body_limit: 1024,
                    response_body_limit: 1024,
                },
            },
        };
        
        // 测试通配符域名匹配
        assert!(config.should_intercept("example.com", 80));
        assert!(config.should_intercept("test.com", 443));
        assert!(config.should_intercept("subdomain.example.com", 80));
        
        // 测试端口不匹配的情况
        assert!(!config.should_intercept("example.com", 8080));
    }
    
    #[test]
    fn test_should_intercept_wildcard_ports() {
        let config = Config {
            proxy: ProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 8888,
            },
            target: TargetConfig {
                domains: vec!["example.com".to_string()],
                ports: vec![0], // 0 表示通配符
            },
            certificates: CertificatesConfig {
                ca_cert: "certs/ca.crt".to_string(),
                ca_key: "certs/ca.key".to_string(),
            },
            logging: LoggingConfig {
                level: "debug".to_string(),
                output: "file".to_string(),
                log_dir: "logs".to_string(),
                program_log: "proxy.log".to_string(),
                domain_logs: DomainLogsConfig {
                    enabled: true,
                    format: "domain_{domain}_{date}.log".to_string(),
                    request_body_limit: 1024,
                    response_body_limit: 1024,
                },
            },
        };
        
        // 测试通配符端口匹配
        assert!(config.should_intercept("example.com", 80));
        assert!(config.should_intercept("example.com", 443));
        assert!(config.should_intercept("example.com", 8080));
        
        // 测试域名不匹配的情况
        assert!(!config.should_intercept("other.com", 80));
    }
    
    #[test]
    fn test_deserialize_ports() {
        let json = r#"{"domains": ["example.com"], "ports": [80, 443, "*"]}"#;
        let target: TargetConfig = serde_json::from_str(json).expect("Failed to deserialize ports");
        
        // 检查普通端口
        assert!(target.ports.contains(&80));
        assert!(target.ports.contains(&443));
        
        // 检查通配符端口（转换为0）
        assert!(target.ports.contains(&0));
    }
    
    #[test]
    fn test_default_limits() {
        assert_eq!(default_request_body_limit(), 1024);
        assert_eq!(default_response_body_limit(), 1024);
    }
}