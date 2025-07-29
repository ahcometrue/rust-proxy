use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use std::io::{BufReader, Cursor};
use std::collections::HashMap;

use crate::config::Config;
use crate::cert::CertManager;
use crate::domain_logger::DomainLogger;
use crate::proxy::response::HttpResponseProcessor;
use crate::proxy::response::ProcessingResult;
use crate::proxy::request::parse_url_and_target;
use crate::proxy::request::log_request_start;

/// 处理TCP连接
/// 
/// # 参数
/// * `stream` - TCP流
/// * `config` - 配置信息
/// * `cert_manager` - 证书管理器
/// * `logger` - 日志记录器
/// 
/// # 返回值
/// 返回Result，如果过程中出现错误则返回错误信息
pub async fn handle_connection(
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
    
    // 检查HTTP请求行是否有效
    match parts.len() {
        len if len < 3 => {
            log::warn!("Invalid HTTP request: {first_line}");
            return Ok(());
        },
        _ => (), // 有效请求行
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
        log::info!("  {line}");
    }
    
    // 记录完整的原始请求
    log::info!("📝 RAW REQUEST:");
    log::info!("{}", String::from_utf8_lossy(&buffer));

    // 根据HTTP方法处理不同类型的请求
    match method {
        "CONNECT" => {
            handle_https_connect(path, stream, config, cert_manager, logger).await?;
        },
        _ => {
            handle_http_request(request_str.to_string(), stream, config, logger).await?;
        }
    }

    Ok(())
}

/// 处理HTTPS CONNECT请求
async fn handle_https_connect(
    path: &str,
    mut client_stream: TcpStream,
    config: Arc<Config>,
    cert_manager: Arc<CertManager>,
    logger: Arc<DomainLogger>,
) -> Result<()> {
    let start_time = Instant::now();
    let parts: Vec<&str> = path.split(':').collect();
    let host = parts[0].to_string();
    let port = parts.get(1).unwrap_or(&"443").parse::<u16>().unwrap_or(443);

    log::info!("🔒 HTTPS CONNECT =========================================");
    log::info!("⏰ Timestamp: {:?}", std::time::SystemTime::now());
    log::info!("🎯 Target: {host}:{port}");
    log::info!("🔍 Intercept: {}", config.should_intercept(&host, port));

    // 记录CONNECT请求
    let duration_ms = start_time.elapsed().as_millis();
    let log_entry = DomainLogger::create_tunnel_log_entry(
            host.clone(),
            duration_ms,
            None,
        );
    logger.log_request(log_entry);

    if !config.should_intercept(&host, port) {
        log::info!("🚇 DIRECT TUNNEL MODE ===================================");
        
        // 发送200 Connection Established
        let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
        client_stream.write_all(response.as_bytes()).await?;

        // 建立直接隧道
        log::info!("Connecting to target server: {host}:{port}");
        let server_stream = TcpStream::connect(format!("{host}:{port}")).await?;
        log::info!("Tunnel established successfully");
        
        let (client_bytes, server_bytes) = tunnel_connection_with_logging(client_stream, server_stream).await?;
        let duration_ms = start_time.elapsed().as_millis();
        log::info!("=== DIRECT TUNNEL CLOSED ===");
        log::info!("Bytes transferred: client={client_bytes}, server={server_bytes}");
        
        // 使用新的DomainLogger记录隧道模式日志
        let log_entry = DomainLogger::create_log_entry(
            host.clone(),
            "CONNECT".to_string(),
            format!("{host}:{port}"),
            HashMap::new(),
            HashMap::new(),
            200,
            String::new(),
            String::new(),
            String::new(),
            duration_ms,
            None,
        );
        logger.log_request(log_entry);
        return Ok(());
    }

    log::info!("=== INTERCEPT MODE ===");
    log::info!("Intercepting HTTPS connection to {host}:{port}");
    
    // 发送200 Connection Established
    let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
    client_stream.write_all(response.as_bytes()).await?;

    // 生成站点证书
    let (cert_pem, key_pem) = cert_manager.generate_site_cert(&host)?;
    log::debug!("Generated site certificate for {host}");

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
            log::info!("TLS handshake successful for {host}");
            stream
        },
        Err(e) => {
            log::error!("TLS handshake failed for {host}: {e}");
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
        log::warn!("Invalid HTTPS request: {first_line}");
        return Ok(());
    }
    
    let method = parts[0];
    let path = parts[1];
    
    log::info!("🌐 HTTPS REQUEST ==========================================");
    log::info!("⏰ Timestamp: {:?}", std::time::SystemTime::now());
    log::info!("📝 Method: {method}");
    log::info!("🔗 Path: {path}");
    log::info!("🌐 Host: {host}:{port}");
    
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
            .map(|(k, v)| format!("{k}={v}"))
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
    let mut new_request = format!("{method} {path} HTTP/1.1\r\n");
    new_request.push_str(&format!("Host: {host}:{port}\r\n"));
    
    // 保留原始头部
    for (key, value) in &headers {
        if key != "host" {
            new_request.push_str(&format!("{key}: {value}\r\n"));
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
    log::info!("Connecting to HTTPS server: {host}:{port}");
    let server_stream = TcpStream::connect(format!("{host}:{port}")).await?;
    
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
    
    // 使用新的响应处理器
    let mut response_processor = HttpResponseProcessor::new(config.logging.domain_logs.response_body_limit);
    let mut buffer = [0; 4096];
    let mut total_response_buffer = Vec::new();

    log::info!("Reading HTTPS response...");

    loop {
        let bytes_read = tls_server_stream.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        let chunk_data = &buffer[..bytes_read];
        total_response_buffer.extend_from_slice(chunk_data);

        // 使用响应处理器处理数据块
        match response_processor.process_chunk_tls(chunk_data, &mut tls_stream).await? {
            ProcessingResult::Continue => continue,
            ProcessingResult::Complete => break,
        }
    }
    
    let duration_ms = start_time.elapsed().as_millis();
    log::info!("✅ HTTPS REQUEST COMPLETE - {} bytes transferred - Duration: {}ms", total_response_buffer.len(), duration_ms);
    
    // 解析响应头和状态码用于日志记录
    let response_str = String::from_utf8_lossy(&total_response_buffer);
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
    if let Some(pos) = total_response_buffer.windows(4).position(|w| w == b"\r\n\r\n") {
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
    let response_body_str = if response_processor.needs_decompression() {
        // 使用解压缩后的响应体
        response_processor.get_decompressed_body()
    } else if header_end > 0 && header_end < total_response_buffer.len() {
        // 使用原始响应体
        String::from_utf8_lossy(&total_response_buffer[header_end..]).to_string()
    } else {
        String::new()
    };
    let log_entry = DomainLogger::create_log_entry(
        host.clone(),
        method.to_string(),
        format!("https://{host}:{port}{path}"),
        request_headers,
        response_headers_map,
        response_status,
        request_body,
        response_body_str,
        url_params,
        duration_ms,
        None,
    );
    logger.log_request(log_entry);
    
    Ok(())
}

/// 处理HTTP请求
async fn handle_http_request(
    request: String,
    mut client_stream: TcpStream,
    config: Arc<Config>,
    logger: Arc<DomainLogger>,
) -> Result<()> {
    let start_time = Instant::now();
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() {
        return Ok(());
    }

    let first_line = lines[0];
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    
    if parts.len() < 3 {
        log::warn!("Invalid HTTP request: {first_line}");
        return Ok(());
    }

    let method = parts[0];
    let url = parts[1];
    let _version = parts[2];

    // 解析URL和目标
    let (host, port, path) = parse_url_and_target(url, &lines)?;

    log::info!("🌐 HTTP REQUEST ==========================================");
    log::info!("⏰ Timestamp: {:?}", std::time::SystemTime::now());
    log::info!("📝 Method: {method}");
    log::info!("🔗 Path: {path}");
    log::info!("🌐 Host: {host}:{port}");
    log::info!("📋 Full Request:");
    log::info!("{request}");

    // 使用新的DomainLogger记录请求日志（异步，不阻塞主流程）
    
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
            .map(|(k, v)| format!("{k}={v}"))
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
        log::info!("Intercepting HTTP request to {host}:{port}{path}");
    }

    // 构建新的HTTP请求，保持原始请求头
    let mut new_request = format!("{method} {path} HTTP/1.1\r\n");
    
    // 计算请求总大小
    let _request_size = new_request.len() + request_body.len();
    
    // 收集并打印原始请求头
    let mut headers_map = HashMap::new();
    for line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            log::info!("📋 Request Header: {key}: {value}");
            headers_map.insert(key, value);
        }
    }
    
    // 设置Host头
    new_request.push_str(&format!("Host: {host}:{port}\r\n"));
    
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
    log::info!("Connecting to target server: {host}:{port}");
    let mut server_stream = TcpStream::connect(format!("{host}:{port}")).await?;
    
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

    // 使用新的响应处理器
    let mut response_processor = HttpResponseProcessor::new(config.logging.domain_logs.response_body_limit);
    let mut buffer = [0; 4096];
    let mut total_response_buffer = Vec::new();
    
    log::info!("Reading HTTP response...");
    
    while let Ok(bytes_read) = server_stream.read(&mut buffer).await {
        if bytes_read == 0 {
            break;
        }
        
        let chunk_data = &buffer[..bytes_read];
        total_response_buffer.extend_from_slice(chunk_data);
        
        // 使用响应处理器处理数据块
        match response_processor.process_chunk_http(chunk_data, &mut client_stream).await? {
            ProcessingResult::Continue => continue,
            ProcessingResult::Complete => break,
        }
    }
    
    // 解析响应头和状态码用于日志记录
    let response_str = String::from_utf8_lossy(&total_response_buffer);
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
    if let Some(pos) = total_response_buffer.windows(4).position(|w| w == b"\r\n\r\n") {
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
    
    // 使用新的DomainLogger记录完整的HTTP请求响应日志
    let response_body_str = if response_processor.needs_decompression() {
        // 使用解压缩后的响应体
        response_processor.get_decompressed_body()
    } else if header_end > 0 && header_end < total_response_buffer.len() {
        // 使用原始响应体
        String::from_utf8_lossy(&total_response_buffer[header_end..]).to_string()
    } else {
        String::new()
    };
    let duration_ms = start_time.elapsed().as_millis();
    let log_entry = DomainLogger::create_log_entry(
        host.clone(),
        method.to_string(),
        format!("http://{host}:{port}{path}"),
        request_headers,
        response_headers_map,
        response_status,
        request_body,
        response_body_str,
        url_params,
        duration_ms,
        None,
    );
    logger.log_request(log_entry);
    
    log::info!("✅ HTTP REQUEST COMPLETE - {} bytes transferred - Duration: {}ms", total_response_buffer.len(), duration_ms);

    Ok(())
}

/// 建立隧道连接并记录日志
async fn tunnel_connection_with_logging(
    client_stream: TcpStream,
    server_stream: TcpStream,
) -> Result<(u64, u64), std::io::Error> {
    let (mut client_reader, mut client_writer) = tokio::io::split(client_stream);
    let (mut server_reader, mut server_writer) = tokio::io::split(server_stream);

    let client_to_server = tokio::io::copy(&mut client_reader, &mut server_writer);
    let server_to_client = tokio::io::copy(&mut server_reader, &mut client_writer);

    let (bytes_client_to_server, bytes_server_to_client) = tokio::try_join!(client_to_server, server_to_client)?;

    Ok((bytes_client_to_server, bytes_server_to_client))
}

/// 加载证书
fn load_certificates(cert_pem: &[u8]) -> Vec<rustls::Certificate> {
    let mut reader = BufReader::new(Cursor::new(cert_pem));
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .into_iter()
        .map(rustls::Certificate)
        .collect()
}

/// 加载私钥
fn load_private_key(key_pem: &[u8]) -> rustls::PrivateKey {
    let mut reader = BufReader::new(Cursor::new(key_pem));
    let key = rustls_pemfile::pkcs8_private_keys(&mut reader)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    rustls::PrivateKey(key)
}