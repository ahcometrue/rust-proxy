use anyhow::Result;
use hyper::Client;
use hyper_tls::HttpsConnector;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig};
use std::io::{BufReader, Cursor};
use std::convert::Infallible;
use std::collections::HashMap;

use crate::config::Config;
use crate::cert::CertManager;
use crate::domain_logger::DomainLogger;

pub struct ProxyServer {
    config: Arc<Config>,
    cert_manager: Arc<CertManager>,
    client: Client<HttpsConnector<hyper::client::HttpConnector>>,
    logger: Arc<DomainLogger>,
}

impl ProxyServer {
    pub fn new(config: Config) -> Result<Self> {
        let cert_manager = CertManager::new(
            &config.certificates.ca_cert,
            &config.certificates.ca_key,
        )?;

        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, hyper::Body>(https);
        let logger = DomainLogger::new(Arc::new(config.clone()));

        Ok(Self {
            config: Arc::new(config),
            cert_manager: Arc::new(cert_manager),
            client,
            logger,
        })
    }

    pub async fn run(self) -> Result<()> {
        let addr = SocketAddr::new(
            self.config.proxy.host.parse().unwrap(),
            self.config.proxy.port,
        );

        let listener = TcpListener::bind(addr).await?;
        log::info!("Proxy server listening on {}", addr);

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            log::info!("New connection from {}", peer_addr);

            let config = Arc::clone(&self.config);
            let cert_manager = Arc::clone(&self.cert_manager);
            let logger = self.logger.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, config, cert_manager, logger).await {
                    log::error!("Connection error: {}", e);
                }
            });
        }
    }
}

fn log_with_timestamp(level: log::Level, message: &str) {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    log::log!(level, "[{}] {}", timestamp, message);
}

fn log_request_start(method: &str, path: &str, host: Option<&str>) {
    log::info!("🔍 REQUEST START ========================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("📝 Method: {}", method);
    log::info!("🔗 Path: {}", path);
    if let Some(h) = host {
        log::info!("🌐 Host: {}", h);
    }
}

fn log_response_summary(bytes: usize, status: Option<&str>) {
    log::info!("📊 RESPONSE SUMMARY ======================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("📦 Response size: {} bytes", bytes);
    if let Some(s) = status {
        log::info!("🎯 Status: {}", s);
    }
    log::info!("✅ REQUEST COMPLETE =====================================");
}

async fn handle_connection(
    mut stream: TcpStream,
    config: Arc<Config>,
    cert_manager: Arc<CertManager>,
    logger: Arc<DomainLogger>,
) -> Result<()> {
    let mut buffer = Vec::new();
    let mut temp_buffer = [0; 1024];
    
    // 读取HTTP头直到找到空行
    loop {
        let bytes_read = stream.read(&mut temp_buffer).await?;
        if bytes_read == 0 {
            return Ok(());
        }
        
        buffer.extend_from_slice(&temp_buffer[..bytes_read]);
        
        // 检查是否找到HTTP头的结束标记 \r\n\r\n
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        
        // 防止读取过多数据
        if buffer.len() > 8192 {
            log::warn!("HTTP header too large");
            return Ok(());
        }
    }

    let request_str = String::from_utf8_lossy(&buffer);
    let lines: Vec<&str> = request_str.lines().collect();
    
    if lines.is_empty() {
        return Ok(());
    }

    let first_line = lines[0];
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    
    if parts.len() < 3 {
        log::warn!("Invalid HTTP request: {}", first_line);
        return Ok(());
    }

    let method = parts[0];
    let path = parts[1];

    // 详细记录请求信息
    let host = lines.iter()
        .find(|line| line.to_lowercase().starts_with("host:"))
        .map(|line| line[5..].trim());
    
    log_request_start(method, path, host);
    
    log::info!("📋 REQUEST HEADERS:");
    for line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        log::info!("  {}", line);
    }
    
    // 记录完整的原始请求
    log::info!("📝 RAW REQUEST:");
    log::info!("{}", String::from_utf8_lossy(&buffer));

    if method == "CONNECT" {
        handle_https_connect(path, stream, config, cert_manager, logger).await?;
    } else {
        handle_http_request(request_str.to_string(), stream, config, logger).await?;
    }

    Ok(())
}



async fn handle_https_connect(
    path: &str,
    mut client_stream: TcpStream,
    config: Arc<Config>,
    cert_manager: Arc<CertManager>,
    logger: Arc<DomainLogger>,
) -> Result<()> {
    let parts: Vec<&str> = path.split(':').collect();
    let host = parts[0].to_string();
    let port = parts.get(1).unwrap_or(&"443").parse::<u16>().unwrap_or(443);

    log::info!("🔒 HTTPS CONNECT =========================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("🎯 Target: {}:{}", host, port);
    log::info!("🔍 Intercept: {}", config.should_intercept(&host, port));

    // 记录CONNECT请求
    let logger_clone = logger.clone();
    let log_entry = DomainLogger::create_tunnel_log_entry(
        host.clone(),
        0,
        0,
        None,
    );
    logger_clone.log_request(log_entry);

    if !config.should_intercept(&host, port) {
        log::info!("🚇 DIRECT TUNNEL MODE ===================================");
        
        // 发送200 Connection Established
        let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
        client_stream.write_all(response.as_bytes()).await?;

        // 建立直接隧道
        log::info!("Connecting to target server: {}:{}", host, port);
        let server_stream = TcpStream::connect(format!("{}:{}", host, port)).await?;
        log::info!("Tunnel established successfully");
        
        let (client_bytes, server_bytes) = tunnel_connection_with_logging(client_stream, server_stream).await?;
        log::info!("=== DIRECT TUNNEL CLOSED ===");
        log::info!("Bytes transferred: client={}, server={}", client_bytes, server_bytes);
        
        // 使用新的DomainLogger记录隧道模式日志
        let logger_clone = logger.clone();
        let log_entry = DomainLogger::create_log_entry(
            host.clone(),
            "CONNECT".to_string(),
            format!("{}:{}", host, port),
            HashMap::new(),
            HashMap::new(),
            200,
            String::new(),
            String::new(),
            String::new(),
            client_bytes as usize,
            server_bytes as usize,
            true, // 标记为隧道模式
            None,
        );
        logger_clone.log_request(log_entry);
        return Ok(());
    }

    log::info!("=== INTERCEPT MODE ===");
    log::info!("Intercepting HTTPS connection to {}:{}", host, port);
    
    // 发送200 Connection Established
    let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
    client_stream.write_all(response.as_bytes()).await?;

    // 生成站点证书
    let (cert_pem, key_pem) = cert_manager.generate_site_cert(&host)?;
    log::debug!("Generated site certificate for {}", host);

    // 创建TLS配置
    let cert_chain = load_certificates(&cert_pem);
    let private_key = load_private_key(&key_pem);
    
    let tls_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)?;

    // 建立TLS连接
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let mut tls_stream = match acceptor.accept(client_stream).await {
        Ok(stream) => {
            log::info!("TLS handshake successful for {}", host);
            stream
        },
        Err(e) => {
            log::error!("TLS handshake failed for {}: {}", host, e);
            return Err(e.into());
        }
    };

    // 对于拦截的HTTPS，使用HTTPS客户端重新建立连接
    log::info!("Processing HTTPS request through intercept mode...");
    
    // 读取完整的HTTPS请求
    let mut request_buffer = Vec::new();
    let mut temp_buffer = [0; 4096];
    
    // 读取请求头直到找到空行
    loop {
        let bytes_read = tls_stream.read(&mut temp_buffer).await?;
        if bytes_read == 0 {
            return Ok(());
        }
        
        request_buffer.extend_from_slice(&temp_buffer[..bytes_read]);
        
        // 检查是否找到HTTP头的结束标记 \r\n\r\n
        if request_buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        
        // 防止读取过多数据
        if request_buffer.len() > 8192 {
            log::warn!("HTTPS request header too large");
            return Ok(());
        }
    }
    
    let request_str = String::from_utf8_lossy(&request_buffer);
    let lines: Vec<&str> = request_str.lines().collect();
    if lines.is_empty() {
        return Ok(());
    }
    
    let first_line = lines[0];
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 3 {
        log::warn!("Invalid HTTPS request: {}", first_line);
        return Ok(());
    }
    
    let method = parts[0];
    let path = parts[1];
    
    log::info!("🌐 HTTPS REQUEST ==========================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("📝 Method: {}", method);
    log::info!("🔗 Path: {}", path);
    log::info!("🌐 Host: {}:{}", host, port);
    
    // 解析请求头和请求体
    let mut headers = HashMap::new();
    let mut request_body = String::new();
    
    for line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            headers.insert(key, value);
        }
    }
    
    // 提取请求体（如果有）
    if let Some(body_start) = request_str.find("\r\n\r\n") {
        request_body = request_str[body_start + 4..].to_string();
    }
    
    // 解析URL参数
    let url_params = if let Some(query_start) = path.find('?') {
        let query = &path[query_start + 1..];
        query.split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    } else {
        String::new()
    };
    
    // 收集请求头
    let request_headers: HashMap<String, String> = lines[1..].iter()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    // 不再提前记录日志，将在获取完整响应信息后记录
    
    // 构建新的HTTP请求
    let mut new_request = format!("{} {} HTTP/1.1\r\n", method, path);
    new_request.push_str(&format!("Host: {}:{}\r\n", host, port));
    
    // 保留原始头部
    for (key, value) in &headers {
        if key != "host" {
            new_request.push_str(&format!("{}: {}\r\n", key, value));
        }
    }
    
    // 添加必要的头部
    if !headers.contains_key("user-agent") {
        new_request.push_str("User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36\r\n");
    }
    if !headers.contains_key("accept") {
        new_request.push_str("Accept: */*\r\n");
    }
    
    new_request.push_str("\r\n");
    
    // 添加请求体
    if !request_body.is_empty() {
        new_request.push_str(&request_body);
    }
    
    // 使用HTTPS连接器建立到目标服务器的连接
    log::info!("Connecting to HTTPS server: {}:{}", host, port);
    let server_stream = TcpStream::connect(format!("{}:{}", host, port)).await?;
    
    // 建立TLS连接
    let connector = tokio_native_tls::TlsConnector::from(
        tokio_native_tls::native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()?,
    );
    
    let mut tls_server_stream = connector
        .connect(&host, server_stream)
        .await?;
    
    log::info!("HTTPS connection established to target server");
    
    // 发送请求
    tls_server_stream.write_all(new_request.as_bytes()).await?;
    
    // 读取并验证HTTPS响应格式
    let mut response_buffer = Vec::new();
    let mut buffer = [0; 4096];
    
    log::info!("Reading HTTPS response...");
    
    let mut first_chunk = true;
    let mut bytes_received = 0;
    
    loop {
        let bytes_read = tls_server_stream.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        
        bytes_received += bytes_read;
        
        // 验证第一块数据是否包含HTTP状态行
        if first_chunk && bytes_read > 0 {
            let chunk_str = String::from_utf8_lossy(&buffer[..bytes_read]);
            if !chunk_str.starts_with("HTTP/") {
                log::warn!("HTTPS response missing HTTP status line, adding HTTP/1.1 200 OK");
                
                // 构建正确的HTTP响应头
                let http_header = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
                tls_stream.write_all(http_header).await?;
                tls_stream.write_all(&buffer[..bytes_read]).await?;
                first_chunk = false;
                continue;
            }
        }
        
        // 正常转发HTTP响应
        tls_stream.write_all(&buffer[..bytes_read]).await?;
        response_buffer.extend_from_slice(&buffer[..bytes_read]);
        first_chunk = false;
    }
    
    let total_bytes = response_buffer.len();
    log::info!("✅ HTTPS REQUEST COMPLETE - {} bytes transferred", total_bytes);
    
    // 解析响应头和状态码用于日志记录
    let response_str = String::from_utf8_lossy(&response_buffer);
    let response_lines: Vec<&str> = response_str.lines().collect();
    let mut response_headers_map = HashMap::new();
    let mut response_status = 0;
    
    if let Some(status_line) = response_lines.first() {
        let status_parts: Vec<&str> = status_line.split_whitespace().collect();
        if status_parts.len() >= 2 {
            response_status = status_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    
    let mut header_end = 0;
    if let Some(pos) = response_buffer.windows(4).position(|w| w == b"\r\n\r\n") {
        header_end = pos + 4;
    }
    
    for line in response_lines.iter().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            response_headers_map.insert(key, value);
        }
    }
    
    // 使用新的DomainLogger记录完整的HTTPS请求响应日志
    let logger_clone = logger.clone();
    let response_body_str = if header_end > 0 && header_end < response_buffer.len() {
        String::from_utf8_lossy(&response_buffer[header_end..]).to_string()
    } else {
        String::new()
    };
    let log_entry = DomainLogger::create_log_entry(
        host.clone(),
        method.to_string(),
        format!("https://{}:{}{}", host, port, path),
        request_headers.clone(),
        response_headers_map,
        response_status,
        request_body.clone(),
        response_body_str,
        url_params.clone(),
        new_request.len(),
        total_bytes,
        false,
        None,
    );
    logger_clone.log_request(log_entry);
    
    Ok(())
}

async fn handle_http_request(
    request: String,
    mut client_stream: TcpStream,
    config: Arc<Config>,
    logger: Arc<DomainLogger>,
) -> Result<()> {
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() {
        return Ok(());
    }

    let first_line = lines[0];
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    
    if parts.len() < 3 {
        log::warn!("Invalid HTTP request: {}", first_line);
        return Ok(());
    }

    let method = parts[0];
    let url = parts[1];
    let _version = parts[2];

    // 解析URL和目标
    let (host, port, path) = if url.starts_with("http://") {
        let url_parts: Vec<&str> = url.splitn(3, '/').collect();
        let host_port = url_parts[2];
        let host_parts: Vec<&str> = host_port.splitn(2, ':').collect();
        let host = host_parts[0].to_string();
        let port = if host_parts.len() > 1 { host_parts[1].parse().unwrap_or(80) } else { 80 };
        let path = if url_parts.len() > 2 {
            format!("/{}", url_parts[2..].join("/"))
        } else {
            "/".to_string()
        };
        (host, port, path)
    } else if url.starts_with("https://") {
        let url_parts: Vec<&str> = url.splitn(3, '/').collect();
        let host_port = url_parts[2];
        let host_parts: Vec<&str> = host_port.splitn(2, ':').collect();
        let host = host_parts[0].to_string();
        let port = if host_parts.len() > 1 { host_parts[1].parse().unwrap_or(443) } else { 443 };
        let path = if url_parts.len() > 2 {
            format!("/{}", url_parts[2..].join("/"))
        } else {
            "/".to_string()
        };
        (host, port, path)
    } else {
        // 绝对路径格式，从Host头获取
        let host_line = lines.iter().find(|line| line.to_lowercase().starts_with("host:"));
        let host = host_line
            .and_then(|line| line.split(':').nth(1))
            .unwrap_or("")
            .trim()
            .to_string();
        let host_parts: Vec<&str> = host.split(':').collect();
        let host = host_parts[0].to_string();
        let port = if host_parts.len() > 1 { host_parts[1].parse().unwrap_or(80) } else { 80 };
        (host, port, url.to_string())
    };

    log::info!("🌐 HTTP REQUEST ==========================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("📝 Method: {}", method);
    log::info!("🔗 Path: {}", path);
    log::info!("🌐 Host: {}:{}", host, port);
    log::info!("📋 Full Request:");
    log::info!("{}", request);

    // 使用新的DomainLogger记录请求日志（异步，不阻塞主流程）
    let logger_clone = logger.clone();
    
    // 收集请求信息
    let request_headers: HashMap<String, String> = lines[1..].iter()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let url_params = if let Some(query_start) = path.find('?') {
        let query = &path[query_start + 1..];
        query.split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    } else {
        String::new()
    };

    let request_body = if let Some(body_start) = request.find("\r\n\r\n") {
        request[body_start + 4..].to_string()
    } else {
        String::new()
    };

    if config.should_intercept(&host, port) {
        log::info!("Intercepting HTTP request to {}:{}{}", host, port, path);
    }

    // 构建新的HTTP请求，保持原始请求头
    let mut new_request = format!("{} {} HTTP/1.1\r\n", method, path);
    
    // 计算请求总大小
    let request_size = new_request.len() + request_body.len();
    
    // 收集并打印原始请求头
    let mut headers_map = HashMap::new();
    for line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            log::info!("📋 Request Header: {}: {}", key, value);
            headers_map.insert(key, value);
        }
    }
    
    // 设置Host头
    new_request.push_str(&format!("Host: {}:{}\r\n", host, port));
    
    // 添加或保留其他必要头部
    if !headers_map.contains_key("user-agent") {
        new_request.push_str("User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36\r\n");
    }
    if !headers_map.contains_key("accept") {
        new_request.push_str("Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8\r\n");
    }
    if !headers_map.contains_key("accept-encoding") {
        new_request.push_str("Accept-Encoding: gzip, deflate, br\r\n");
    }
    if !headers_map.contains_key("accept-language") {
        new_request.push_str("Accept-Language: zh-CN,zh;q=0.9,en;q=0.8\r\n");
    }
    
    // 保留原始头部
    for line in &lines[1..] {
        if !line.is_empty() && !line.to_lowercase().starts_with("host:") {
            new_request.push_str(line);
            new_request.push_str("\r\n");
        }
    }
    new_request.push_str("\r\n");

    // 连接到目标服务器
    log::info!("Connecting to target server: {}:{}", host, port);
    let mut server_stream = TcpStream::connect(format!("{}:{}", host, port)).await?;
    
    // 转发请求
    log::info!("Forwarding request to server...");
    server_stream.write_all(new_request.as_bytes()).await?;

    // 转发请求体（如果有）
    if let Some(body_start) = request.find("\r\n\r\n") {
        let body = &request[body_start + 4..];
        if !body.is_empty() {
            log::info!("Forwarding request body ({} bytes)", body.len());
            server_stream.write_all(body.as_bytes()).await?;
        }
    }

    // 读取整个响应到缓冲区
    let mut total_bytes = 0;
    let mut response_buffer = Vec::new();
    let mut buffer = [0; 4096];
    
    log::info!("Reading response...");
    loop {
        let bytes_read = server_stream.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        response_buffer.extend_from_slice(&buffer[..bytes_read]);
        total_bytes += bytes_read;
    }
    
    // 验证并修复HTTP响应格式
    if !response_buffer.is_empty() {
        let response_str = String::from_utf8_lossy(&response_buffer);
        
        // 检查是否以HTTP状态行开始
        if !response_str.starts_with("HTTP/") {
            log::warn!("Response missing HTTP status line, wrapping with HTTP/1.1 200 OK");
            
            // 构建正确的HTTP响应
            let mut fixed_response = Vec::new();
            fixed_response.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
            fixed_response.extend_from_slice(b"Content-Type: text/plain\r\n");
            fixed_response.extend_from_slice(b"Content-Length: ");
            fixed_response.extend_from_slice(response_buffer.len().to_string().as_bytes());
            fixed_response.extend_from_slice(b"\r\n");
            fixed_response.extend_from_slice(b"Connection: close\r\n");
            fixed_response.extend_from_slice(b"\r\n");
            fixed_response.extend_from_slice(&response_buffer);
            
            client_stream.write_all(&fixed_response).await?;
            total_bytes = fixed_response.len();
        } else {
            // 响应格式正确，直接转发
            client_stream.write_all(&response_buffer).await?;
        }
    }
    
    // 解析响应头
    let response_str = String::from_utf8_lossy(&response_buffer);
    let response_lines: Vec<&str> = response_str.lines().collect();
    let mut response_headers_map = HashMap::new();
    let mut response_status = None;
    let mut status_line = String::new();
    
    if let Some(status_line_str) = response_lines.first() {
        status_line = status_line_str.to_string();
        let status_parts: Vec<&str> = status_line_str.split_whitespace().collect();
        if status_parts.len() >= 2 {
            response_status = status_parts.get(1).and_then(|s| s.parse().ok());
        }
    }

    // 查找响应头结束位置
    let mut header_end = 0;
    if let Some(pos) = response_buffer.windows(4).position(|w| w == b"\r\n\r\n") {
        header_end = pos + 4;
    }
    
    for line in response_lines.iter().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            response_headers_map.insert(key, value);
        }
    }
    
    let response_body = if header_end > 0 && header_end < response_buffer.len() {
        response_buffer[header_end..].to_vec()
    } else {
        Vec::new()
    };
    
    let status = status_line.split_whitespace().nth(1).unwrap_or("Unknown");
    log_response_summary(total_bytes, Some(status));
    log::info!("Forwarding response to client...");
    log::info!("✅ HTTP REQUEST COMPLETE =====================================");

    // 收集请求头
    let request_headers: HashMap<String, String> = lines[1..].iter()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    // 解析URL参数
    let url_params = if let Some(query_start) = path.find('?') {
        let query = &path[query_start + 1..];
        query.split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    } else {
        String::new()
    };

    // 提取请求体
    let request_body = if let Some(body_start) = request.find("\r\n\r\n") {
        request[body_start + 4..].to_string()
    } else {
        String::new()
    };



    // 使用新的DomainLogger记录完整的HTTP请求响应日志
    let logger_clone = logger.clone();
    let response_body_str = if !response_body.is_empty() {
        String::from_utf8_lossy(&response_body).to_string()
    } else {
        String::new()
    };
    let log_entry = DomainLogger::create_log_entry(
        host.clone(),
        method.to_string(),
        format!("http://{}:{}{}", host, port, path),
        request_headers,
        response_headers_map,
        response_status.unwrap_or(0),
        request_body,
        response_body_str,
        url_params,
        request_size, // 使用已计算的request_size
        total_bytes,
        false,
        None,
    );
    logger_clone.log_request(log_entry);

    Ok(())
}

async fn tunnel_connection(
    client: impl AsyncReadExt + AsyncWriteExt + Unpin + Send,
    server: impl AsyncReadExt + AsyncWriteExt + Unpin + Send,
) -> Result<(), Infallible> {
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let (mut server_reader, mut server_writer) = tokio::io::split(server);

    log::info!("Starting bidirectional tunnel...");
    
    let client_to_server = tokio::io::copy(&mut client_reader, &mut server_writer);
    let server_to_client = tokio::io::copy(&mut server_reader, &mut client_writer);

    match tokio::try_join!(client_to_server, server_to_client) {
        Ok((bytes_up, bytes_down)) => {
            log::info!("Tunnel closed successfully");
            log::info!("Bytes transferred: client→server={}, server→client={}", bytes_up, bytes_down);
        }
        Err(e) => {
            log::error!("Tunnel error: {}", e);
        }
    }

    Ok(())
}

async fn tunnel_connection_with_logging<T, U>(
    client: T,
    server: U,
) -> Result<(u64, u64)>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    U: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let (mut server_reader, mut server_writer) = tokio::io::split(server);

    log::info!("Starting tunnel with detailed logging...");
    
    let client_to_server = tokio::io::copy(&mut client_reader, &mut server_writer);
    let server_to_client = tokio::io::copy(&mut server_reader, &mut client_writer);

    match tokio::try_join!(client_to_server, server_to_client) {
        Ok((bytes_up, bytes_down)) => {
            log::info!("Tunnel closed successfully");
            log::info!("Bytes transferred: client→server={}, server→client={}", bytes_up, bytes_down);
            Ok((bytes_up, bytes_down))
        }
        Err(e) => {
            log::error!("Tunnel error: {}", e);
            Ok((0, 0))
        }
    }
}



fn load_certificates(cert_pem: &[u8]) -> Vec<rustls::Certificate> {
    let mut reader = BufReader::new(Cursor::new(cert_pem));
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .into_iter()
        .map(rustls::Certificate)
        .collect()
}

fn load_private_key(key_pem: &[u8]) -> rustls::PrivateKey {
    let mut reader = BufReader::new(Cursor::new(key_pem));
    let key = rustls_pemfile::pkcs8_private_keys(&mut reader)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    rustls::PrivateKey(key)
}