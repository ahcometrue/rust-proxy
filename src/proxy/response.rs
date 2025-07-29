use anyhow::Result;
use std::io::Read;
use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use flate2::read::GzDecoder;

/// 处理结果枚举
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessingResult {
    Continue,
    Complete,
}

/// HTTP响应处理器，用于正确处理各种HTTP响应格式
#[derive(Debug)]
pub struct HttpResponseProcessor {
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
    pub fn new(response_body_limit: i64) -> Self {
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
    pub async fn process_chunk_tls(
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
    pub async fn process_chunk_http(
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
    pub fn needs_decompression(&self) -> bool {
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
    pub fn get_decompressed_body(&self) -> String {
        String::from_utf8_lossy(&self.decompressed_body).to_string()
    }
}