use serde::{Deserialize, Serialize};
use std::fs;
use serde::de::{self, Visitor};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    pub domains: Vec<String>,
    #[serde(deserialize_with = "deserialize_ports")]
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificatesConfig {
    pub ca_cert: String,
    pub ca_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainLogsConfig {
    pub enabled: bool,
    pub format: String,
    #[serde(default = "default_body_limit")]
    pub request_body_limit: i64,
    #[serde(default = "default_body_limit")]
    pub response_body_limit: i64,
}

fn default_body_limit() -> i64 {
    1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub output: String,
    pub log_dir: String,
    pub program_log: String,
    pub domain_logs: DomainLogsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub proxy: ProxyConfig,
    pub target: TargetConfig,
    pub certificates: CertificatesConfig,
    pub logging: LoggingConfig,
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn should_intercept(&self, domain: &str, port: u16) -> bool {
        let domain_match = self.target.domains.iter().any(|d| {
            d == "*" || domain.contains(d)
        });
        let port_match = self.target.ports.iter().any(|p| {
            *p == 0 || *p == port
        });
        domain_match && port_match
    }
}

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
                        if s == "*" {
                            ports.push(0); // 0 表示通配符
                        } else if let Ok(port) = s.parse::<u16>() {
                            ports.push(port);
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