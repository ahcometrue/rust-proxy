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