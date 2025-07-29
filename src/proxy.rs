use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig};
use std::io::{BufReader, Cursor};
use std::collections::HashMap;
use flate2::read::GzDecoder;
use std::io::Read;

use crate::config::Config;
use crate::cert::CertManager;
use crate::domain_logger::DomainLogger;

/// HTTP响应处理器，用于正确处理各种HTTP响应格式
#[derive(Debug)]
struct HttpResponseProcessor {
    /// 响应头是否已解析
    headers_parsed: bool,
    /// 响应头结束位置
    header_end: Option<usize>,
    /// Content-Length
    content_length: Option<usize>,
    /// Transfer-Encoding
    transfer_encoding: Option<String>,
    /// Content-Encoding (gzip, deflate等)
    content_encoding: Option<String>,
    /// Connection类型
    connection: Option<String>,
    /// 当前chunk解析状态
    chunk_state: ChunkState,
    /// 已转发的数据长度
    forwarded_bytes: usize,
    /// 解压缩后的响应体（用于日志记录）
    decompressed_body: Vec<u8>,
    /// 响应体记录长度限制
    response_body_limit: i64,
}

#[derive(Debug)]
enum ChunkState {
    /// 等待chunk大小
    WaitingSize,
    /// 等待chunk数据
    WaitingData { remaining: usize },
    /// 等待chunk结束符\r\n
    WaitingEnd,
    /// 等待终止chunk (0\r\n\r\n)
    WaitingTerminator,
    /// 完成
    Complete,
}

impl HttpResponseProcessor {
    fn new(response_body_limit: i64) -> Self {
        Self {
            headers_parsed: false,
            header_end: None,
            content_length: None,
            transfer_encoding: None,
            content_encoding: None,
            connection: None,
            chunk_state: ChunkState::WaitingSize,
            forwarded_bytes: 0,
            decompressed_body: Vec::new(),
            response_body_limit,
        }
    }

    /// 按limit累积响应体
    fn accumulate_body(&mut self, data: &[u8]) {
        if self.response_body_limit == 0 {
            return;
        }
        if self.response_body_limit < 0 {
            self.decompressed_body.extend_from_slice(data);
        } else {
            let remain = self.response_body_limit as usize - self.decompressed_body.len();
            if remain > 0 {
                let to_copy = remain.min(data.len());
                self.decompressed_body.extend_from_slice(&data[..to_copy]);
            }
        }
    }

    /// 处理响应数据块（TLS版本）
    async fn process_chunk_tls(
        &mut self,
        data: &[u8],
        client_stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<ProcessingResult> {
        if !self.headers_parsed {
            return self.process_headers_tls(data, client_stream).await;
        }
        
        match self.transfer_encoding.as_deref() {
            Some("chunked") => self.process_chunked_body_tls(data, client_stream).await,
            _ => self.process_normal_body_tls(data, client_stream).await,
        }
    }

    /// 处理响应数据块（HTTP版本）
    async fn process_chunk_http(
        &mut self,
        data: &[u8],
        client_stream: &mut TcpStream,
    ) -> Result<ProcessingResult> {
        if !self.headers_parsed {
            return self.process_headers_http(data, client_stream).await;
        }
        
        match self.transfer_encoding.as_deref() {
            Some("chunked") => self.process_chunked_body_http(data, client_stream).await,
            _ => self.process_normal_body_http(data, client_stream).await,
        }
    }

    /// 处理响应头（TLS版本）
    async fn process_headers_tls(
        &mut self,
        data: &[u8],
        client_stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<ProcessingResult> {
        // 查找响应头结束标记
        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_end = header_end + 4;
            self.header_end = Some(header_end);
            self.headers_parsed = true;
            
            // 解析响应头
            let headers_str = String::from_utf8_lossy(&data[..header_end]);
            self.parse_headers(&headers_str)?;
            
            // 立即转发响应头
            client_stream.write_all(&data[..header_end]).await?;
            self.forwarded_bytes += header_end;
            
            // 如果有响应体，继续处理
            if header_end < data.len() {
                let body_data = &data[header_end..];
                return self.process_body_after_headers_tls(body_data, client_stream).await;
            }
            
            return Ok(ProcessingResult::Continue);
        }
        
        // 还没找到完整的响应头，继续读取
        Ok(ProcessingResult::Continue)
    }

    /// 处理响应头（HTTP版本）
    async fn process_headers_http(
        &mut self,
        data: &[u8],
        client_stream: &mut TcpStream,
    ) -> Result<ProcessingResult> {
        // 查找响应头结束标记
        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_end = header_end + 4;
            self.header_end = Some(header_end);
            self.headers_parsed = true;
            
            // 解析响应头
            let headers_str = String::from_utf8_lossy(&data[..header_end]);
            self.parse_headers(&headers_str)?;
            
            // 立即转发响应头
            client_stream.write_all(&data[..header_end]).await?;
            self.forwarded_bytes += header_end;
            
            // 如果有响应体，继续处理
            if header_end < data.len() {
                let body_data = &data[header_end..];
                return self.process_body_after_headers_http(body_data, client_stream).await;
            }
            
            return Ok(ProcessingResult::Continue);
        }
        
        // 还没找到完整的响应头，继续读取
        Ok(ProcessingResult::Continue)
    }

    /// 解析响应头
    fn parse_headers(&mut self, headers_str: &str) -> Result<()> {
        let lines: Vec<&str> = headers_str.lines().collect();
        
        for line in &lines[1..] {
            if line.is_empty() {
                break;
            }
            
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_lowercase();
                let value = line[colon_pos + 1..].trim().to_string();
                
                match key.as_str() {
                    "content-length" => {
                        self.content_length = value.parse().ok();
                    },
                    "transfer-encoding" => {
                        self.transfer_encoding = Some(value);
                    },
                    "content-encoding" => {
                        self.content_encoding = Some(value);
                    },
                    "connection" => {
                        self.connection = Some(value);
                    },
                    _ => {}
                }
            }
        }
        
        Ok(())
    }

    /// 处理响应头后的数据（TLS版本）
    async fn process_body_after_headers_tls(
        &mut self,
        data: &[u8],
        client_stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<ProcessingResult> {
        match self.transfer_encoding.as_deref() {
            Some("chunked") => self.process_chunked_body_tls(data, client_stream).await,
            _ => self.process_normal_body_tls(data, client_stream).await,
        }
    }

    /// 处理响应头后的数据（HTTP版本）
    async fn process_body_after_headers_http(
        &mut self,
        data: &[u8],
        client_stream: &mut TcpStream,
    ) -> Result<ProcessingResult> {
        match self.transfer_encoding.as_deref() {
            Some("chunked") => self.process_chunked_body_http(data, client_stream).await,
            _ => self.process_normal_body_http(data, client_stream).await,
        }
    }

    /// 处理普通响应体（非chunked，TLS版本）
    async fn process_normal_body_tls(
        &mut self,
        data: &[u8],
        client_stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<ProcessingResult> {
        // 处理压缩数据（用于日志记录）
        self.process_compressed_data(data)?;
        
        if let Some(content_length) = self.content_length {
            // 有Content-Length的情况
            let remaining = content_length.saturating_sub(self.forwarded_bytes);
            let to_forward = std::cmp::min(remaining, data.len());
            
            if to_forward > 0 {
                client_stream.write_all(&data[..to_forward]).await?;
                self.forwarded_bytes += to_forward;
            }
            
            if self.forwarded_bytes >= content_length {
                Ok(ProcessingResult::Complete)
            } else {
                Ok(ProcessingResult::Continue)
            }
        } else if self.connection.as_deref() == Some("close") {
            // Connection: close，转发所有数据并继续读取
            client_stream.write_all(data).await?;
            self.forwarded_bytes += data.len();
            Ok(ProcessingResult::Continue)
        } else {
            // 没有Content-Length且不是close，转发所有数据
            client_stream.write_all(data).await?;
            self.forwarded_bytes += data.len();
            Ok(ProcessingResult::Continue)
        }
    }

    /// 处理普通响应体（非chunked，HTTP版本）
    async fn process_normal_body_http(
        &mut self,
        data: &[u8],
        client_stream: &mut TcpStream,
    ) -> Result<ProcessingResult> {
        // 处理压缩数据（用于日志记录）
        self.process_compressed_data(data)?;
        
        if let Some(content_length) = self.content_length {
            // 有Content-Length的情况
            let remaining = content_length.saturating_sub(self.forwarded_bytes);
            let to_forward = std::cmp::min(remaining, data.len());
            
            if to_forward > 0 {
                client_stream.write_all(&data[..to_forward]).await?;
                self.forwarded_bytes += to_forward;
            }
            
            if self.forwarded_bytes >= content_length {
                Ok(ProcessingResult::Complete)
            } else {
                Ok(ProcessingResult::Continue)
            }
        } else if self.connection.as_deref() == Some("close") {
            // Connection: close，转发所有数据并继续读取
            client_stream.write_all(data).await?;
            self.forwarded_bytes += data.len();
            Ok(ProcessingResult::Continue)
        } else {
            // 没有Content-Length且不是close，转发所有数据
            client_stream.write_all(data).await?;
            self.forwarded_bytes += data.len();
            Ok(ProcessingResult::Continue)
        }
    }

    /// 处理chunked响应体（TLS版本）
    async fn process_chunked_body_tls(
        &mut self,
        data: &[u8],
        client_stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<ProcessingResult> {
        let mut pos = 0;
        
        while pos < data.len() {
            match &mut self.chunk_state {
                ChunkState::WaitingSize => {
                    // 查找chunk大小行结束符
                    if let Some(end_pos) = data[pos..].windows(2).position(|w| w == b"\r\n") {
                        let size_line = &data[pos..pos + end_pos];
                        let size_str = String::from_utf8_lossy(size_line);
                        
                        // 解析chunk大小
                        if let Ok(size) = usize::from_str_radix(size_str.trim(), 16) {
                            if size == 0 {
                                // 终止chunk，检查是否有\r\n\r\n
                                let remaining = &data[pos + end_pos + 2..];
                                if remaining.len() >= 2 && remaining[..2] == *b"\r\n" {
                                    // 完整的终止chunk
                                    client_stream.write_all(&data[pos..pos + end_pos + 4]).await?;
                                    return Ok(ProcessingResult::Complete);
                                } else {
                                    // 等待更多数据
                                    client_stream.write_all(&data[pos..]).await?;
                                    self.chunk_state = ChunkState::WaitingTerminator;
                                    return Ok(ProcessingResult::Continue);
                                }
                            } else {
                                // 非零chunk
                                client_stream.write_all(&data[pos..pos + end_pos + 2]).await?;
                                self.chunk_state = ChunkState::WaitingData { remaining: size };
                                pos += end_pos + 2;
                            }
                        } else {
                            log::warn!("Invalid chunk size: {}", size_str);
                            return Ok(ProcessingResult::Complete);
                        }
                    } else {
                        // 没有找到完整的chunk大小行
                        client_stream.write_all(&data[pos..]).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingData { remaining } => {
                    let to_forward = std::cmp::min(*remaining, data.len() - pos);
                    let chunk_data = &data[pos..pos + to_forward];
                    
                    // 处理压缩数据（用于日志记录）
                    Self::process_compressed_data_static(
                        &self.content_encoding,
                        &mut self.decompressed_body,
                        chunk_data,
                        self.response_body_limit,
                    )?;
                    
                    client_stream.write_all(chunk_data).await?;
                    pos += to_forward;
                    *remaining -= to_forward;
                    
                    if *remaining == 0 {
                        self.chunk_state = ChunkState::WaitingEnd;
                    } else {
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingEnd => {
                    if pos + 1 < data.len() && data[pos..pos + 2] == *b"\r\n" {
                        client_stream.write_all(&data[pos..pos + 2]).await?;
                        pos += 2;
                        self.chunk_state = ChunkState::WaitingSize;
                    } else {
                        // 等待更多数据
                        client_stream.write_all(&data[pos..]).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingTerminator => {
                    // 等待终止chunk的\r\n\r\n
                    if data.len() >= 2 && data[..2] == *b"\r\n" {
                        client_stream.write_all(&data[..2]).await?;
                        return Ok(ProcessingResult::Complete);
                    } else {
                        client_stream.write_all(data).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::Complete => {
                    return Ok(ProcessingResult::Complete);
                }
            }
        }
        
        Ok(ProcessingResult::Continue)
    }

    /// 处理chunked响应体（HTTP版本）
    async fn process_chunked_body_http(
        &mut self,
        data: &[u8],
        client_stream: &mut TcpStream,
    ) -> Result<ProcessingResult> {
        let mut pos = 0;
        
        while pos < data.len() {
            match &mut self.chunk_state {
                ChunkState::WaitingSize => {
                    // 查找chunk大小行结束符
                    if let Some(end_pos) = data[pos..].windows(2).position(|w| w == b"\r\n") {
                        let size_line = &data[pos..pos + end_pos];
                        let size_str = String::from_utf8_lossy(size_line);
                        
                        // 解析chunk大小
                        if let Ok(size) = usize::from_str_radix(size_str.trim(), 16) {
                            if size == 0 {
                                // 终止chunk，检查是否有\r\n\r\n
                                let remaining = &data[pos + end_pos + 2..];
                                if remaining.len() >= 2 && remaining[..2] == *b"\r\n" {
                                    // 完整的终止chunk
                                    client_stream.write_all(&data[pos..pos + end_pos + 4]).await?;
                                    return Ok(ProcessingResult::Complete);
                                } else {
                                    // 等待更多数据
                                    client_stream.write_all(&data[pos..]).await?;
                                    self.chunk_state = ChunkState::WaitingTerminator;
                                    return Ok(ProcessingResult::Continue);
                                }
                            } else {
                                // 非零chunk
                                client_stream.write_all(&data[pos..pos + end_pos + 2]).await?;
                                self.chunk_state = ChunkState::WaitingData { remaining: size };
                                pos += end_pos + 2;
                            }
                        } else {
                            log::warn!("Invalid chunk size: {}", size_str);
                            return Ok(ProcessingResult::Complete);
                        }
                    } else {
                        // 没有找到完整的chunk大小行
                        client_stream.write_all(&data[pos..]).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingData { remaining } => {
                    let to_forward = std::cmp::min(*remaining, data.len() - pos);
                    let chunk_data = &data[pos..pos + to_forward];
                    
                    // 处理压缩数据（用于日志记录）
                    Self::process_compressed_data_static(
                        &self.content_encoding,
                        &mut self.decompressed_body,
                        chunk_data,
                        self.response_body_limit,
                    )?;
                    
                    client_stream.write_all(chunk_data).await?;
                    pos += to_forward;
                    *remaining -= to_forward;
                    
                    if *remaining == 0 {
                        self.chunk_state = ChunkState::WaitingEnd;
                    } else {
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingEnd => {
                    if pos + 1 < data.len() && data[pos..pos + 2] == *b"\r\n" {
                        client_stream.write_all(&data[pos..pos + 2]).await?;
                        pos += 2;
                        self.chunk_state = ChunkState::WaitingSize;
                    } else {
                        // 等待更多数据
                        client_stream.write_all(&data[pos..]).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::WaitingTerminator => {
                    // 等待终止chunk的\r\n\r\n
                    if data.len() >= 2 && data[..2] == *b"\r\n" {
                        client_stream.write_all(&data[..2]).await?;
                        return Ok(ProcessingResult::Complete);
                    } else {
                        client_stream.write_all(data).await?;
                        return Ok(ProcessingResult::Continue);
                    }
                },
                
                ChunkState::Complete => {
                    return Ok(ProcessingResult::Complete);
                }
            }
        }
        
        Ok(ProcessingResult::Continue)
    }

    /// 检查是否需要处理压缩内容
    fn needs_decompression(&self) -> bool {
        matches!(self.content_encoding.as_deref(), Some("gzip") | Some("deflate"))
    }

    /// 解压缩gzip数据
    fn decompress_gzip(&mut self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = GzDecoder::new(compressed_data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(decompressed)
    }

    /// 处理压缩内容并更新解压缩缓冲区
    fn process_compressed_data(&mut self, data: &[u8]) -> Result<()> {
        if !self.needs_decompression() {
            self.accumulate_body(data);
            return Ok(());
        }

        match self.content_encoding.as_deref() {
            Some("gzip") => {
                // 解压缩gzip数据
                let decompressed = self.decompress_gzip(data)?;
                let decompressed_len = decompressed.len();
                self.accumulate_body(&decompressed);
                log::info!("Decompressed {} bytes to {} bytes", data.len(), decompressed_len);
            },
            Some("deflate") => {
                // TODO: 实现deflate解压缩
                log::warn!("Deflate decompression not yet implemented");
                self.accumulate_body(data);
            },
            _ => {
                // 非压缩数据，直接添加
                self.accumulate_body(data);
            }
        }
        Ok(())
    }

    /// 处理压缩数据（静态方法，避免借用冲突）
    fn process_compressed_data_static(
        content_encoding: &Option<String>,
        decompressed_body: &mut Vec<u8>,
        data: &[u8],
        response_body_limit: i64,
    ) -> Result<()> {
        if !matches!(content_encoding.as_deref(), Some("gzip") | Some("deflate")) {
            // 非压缩数据，直接添加
            if response_body_limit == 0 {
                return Ok(());
            }
            if response_body_limit < 0 {
                decompressed_body.extend(data);
            } else {
                let remain = response_body_limit as usize - decompressed_body.len();
                if remain > 0 {
                    let to_copy = remain.min(data.len());
                    decompressed_body.extend(&data[..to_copy]);
                }
            }
            return Ok(());
        }

        match content_encoding.as_deref() {
            Some("gzip") => {
                // 解压缩gzip数据
                let mut decoder = GzDecoder::new(data);
                let mut decompressed = Vec::new();
                decoder.read_to_end(&mut decompressed)?;
                let decompressed_len = decompressed.len();
                if response_body_limit == 0 {
                    // 不记录
                } else if response_body_limit < 0 {
                    decompressed_body.extend(decompressed);
                } else {
                    let remain = response_body_limit as usize - decompressed_body.len();
                    if remain > 0 {
                        let to_copy = remain.min(decompressed.len());
                        decompressed_body.extend(&decompressed[..to_copy]);
                    }
                }
                log::info!("Decompressed {} bytes to {} bytes", data.len(), decompressed_len);
            },
            Some("deflate") => {
                // TODO: 实现deflate解压缩
                log::warn!("Deflate decompression not yet implemented");
                if response_body_limit == 0 {
                } else if response_body_limit < 0 {
                    decompressed_body.extend_from_slice(data);
                } else {
                    let remain = response_body_limit as usize - decompressed_body.len();
                    if remain > 0 {
                        let to_copy = remain.min(data.len());
                        decompressed_body.extend_from_slice(&data[..to_copy]);
                    }
                }
            },
            _ => {
                // 非压缩数据，直接添加
                if response_body_limit == 0 {
                } else if response_body_limit < 0 {
                    decompressed_body.extend_from_slice(data);
                } else {
                    let remain = response_body_limit as usize - decompressed_body.len();
                    if remain > 0 {
                        let to_copy = remain.min(data.len());
                        decompressed_body.extend_from_slice(&data[..to_copy]);
                    }
                }
            }
        }
        Ok(())
    }

    /// 获取解压缩后的响应体
    fn get_decompressed_body(&self) -> String {
        String::from_utf8_lossy(&self.decompressed_body).to_string()
    }
}

/// 代理服务器主结构体
pub struct ProxyServer {
    /// 配置信息
    config: Arc<Config>,
    /// 证书管理器
    cert_manager: Arc<CertManager>,
    /// 日志记录器
    logger: Arc<DomainLogger>,
}

impl ProxyServer {
    /// 创建新的代理服务器实例
    /// 
    /// # 参数
    /// * `config` - 配置信息
    /// 
    /// # 返回值
    /// 返回Result包装的ProxyServer实例，如果过程中出现错误则返回错误信息
    pub fn new(config: Config) -> Result<Self> {
        let cert_manager = CertManager::new(
            &config.certificates.ca_cert,
            &config.certificates.ca_key,
            &config.certificates.name,
        )?;

        let logger = DomainLogger::new(Arc::new(config.clone()));

        Ok(Self {
            config: Arc::new(config),
            cert_manager: Arc::new(cert_manager),
            logger,
        })
    }

    /// 运行代理服务器
    /// 
    /// # 返回值
    /// 返回Result，如果过程中出现错误则返回错误信息
    pub async fn run(self) -> Result<()> {
        let addr = SocketAddr::new(
            self.config.proxy.host.parse().unwrap(),
            self.config.proxy.port,
        );

        let listener = TcpListener::bind(addr).await?;
        log::info!("Proxy server listening on {addr}");

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            log::info!("New connection from {peer_addr}");

            let config = Arc::clone(&self.config);
            let cert_manager = Arc::clone(&self.cert_manager);
            let logger = self.logger.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, config, cert_manager, logger).await {
                    log::error!("Connection error: {e}");
                }
            });
        }
    }
}


/// 记录请求开始日志
fn log_request_start(method: &str, path: &str, host: Option<&str>) {
    log::info!("🔍 REQUEST START ========================================");
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
    log::info!("📝 Method: {method}");
    log::info!("🔗 Path: {path}");
    if let Some(host) = host {
        log::info!("🌐 Host: {host}");
    }
}



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
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
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
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
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
    log::info!("⏰ Timestamp: {:?}", SystemTime::now());
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProcessingResult {
    Continue,
    Complete,
}





fn parse_url_and_target(url: &str, lines: &[&str]) -> Result<(String, u16, String)> {
    let url_info = UrlInfo::parse(url, lines)?;
    Ok((url_info.host, url_info.port, url_info.path))
}

#[derive(Debug)]
struct UrlInfo {
    host: String,
    port: u16,
    path: String,
}

impl UrlInfo {
    fn parse(url: &str, lines: &[&str]) -> Result<Self> {
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

    fn parse_absolute_url(url: &str, scheme: &str) -> Result<Self> {
        let url_parts: Vec<&str> = url.splitn(3, '/').collect();
        let host_port = url_parts.get(2).unwrap_or(&"");
        
        let (host, port) = Self::parse_host_port(host_port, scheme)?;
        let path = Self::build_path(&url_parts[2..]);
        
        Ok(UrlInfo { host, port, path })
    }

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

    fn build_path(url_parts: &[&str]) -> String {
        if url_parts.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", url_parts.join("/"))
        }
    }
}

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