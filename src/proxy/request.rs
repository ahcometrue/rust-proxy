use anyhow::Result;

/// 解析URL和目标信息
pub fn parse_url_and_target(url: &str, lines: &[&str]) -> Result<(String, u16, String)> {
    let url_info = UrlInfo::parse(url, lines)?;
    Ok((url_info.host, url_info.port, url_info.path))
}

/// URL信息结构体
#[derive(Debug)]
pub struct UrlInfo {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl UrlInfo {
    /// 解析URL
    pub fn parse(url: &str, lines: &[&str]) -> Result<Self> {
        let scheme = if url.starts_with("http://") {
            "http"
        } else if url.starts_with("https://") {
            "https"
        } else {
            "relative"
        };

        match scheme {
            "http" | "https" => Self::parse_absolute_url(url, scheme),
            "relative" => Self::parse_relative_url(url, lines),
            _ => unreachable!(),
        }
    }

    /// 解析绝对URL
    fn parse_absolute_url(url: &str, scheme: &str) -> Result<Self> {
        let url_parts: Vec<&str> = url.splitn(3, '/').collect();
        let host_port = url_parts.get(2).unwrap_or(&"");
        
        let (host, port) = Self::parse_host_port(host_port, scheme)?;
        let path = Self::build_path(&url_parts[2..]);
        
        Ok(UrlInfo { host, port, path })
    }

    /// 解析相对URL
    fn parse_relative_url(url: &str, lines: &[&str]) -> Result<Self> {
        let host_line = lines
            .iter()
            .find(|line| line.to_lowercase().starts_with("host:"))
            .ok_or_else(|| anyhow::anyhow!("Missing Host header"))?;

        let host_info = host_line[5..].trim();
        let (host, port) = Self::parse_host_port(host_info, "http")?;
        
        Ok(UrlInfo {
            host,
            port,
            path: url.to_string(),
        })
    }

    /// 解析主机和端口
    fn parse_host_port(host_port: &str, scheme: &str) -> Result<(String, u16)> {
        let parts: Vec<&str> = host_port.splitn(2, ':').collect();
        let host = parts[0].to_string();
        let default_port = match scheme {
            "http" => 80,
            "https" => 443,
            _ => 80,
        };
        
        let port = parts
            .get(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
            
        Ok((host, port))
    }

    /// 构建路径
    fn build_path(url_parts: &[&str]) -> String {
        if url_parts.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", url_parts.join("/"))
        }
    }
}

/// 记录请求开始日志
pub fn log_request_start(method: &str, path: &str, host: Option<&str>) {
    log::info!("🔍 REQUEST START ========================================");
    log::info!("⏰ Timestamp: {:?}", std::time::SystemTime::now());
    log::info!("📝 Method: {method}");
    log::info!("🔗 Path: {path}");
    if let Some(host) = host {
        log::info!("🌐 Host: {host}");
    }
}