use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;

use crate::config::Config;
use crate::cert::CertManager;
use crate::domain_logger::DomainLogger;
use crate::proxy::handler::handle_connection;

/// 代理服务器
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
    /// * `cert_manager` - 证书管理器
    /// * `logger` - 日志记录器
    /// 
    /// # 返回值
    /// 返回新的代理服务器实例
    pub fn new(
        config: Arc<Config>,
        cert_manager: Arc<CertManager>,
        logger: Arc<DomainLogger>,
    ) -> Self {
        Self {
            config,
            cert_manager,
            logger,
        }
    }

    /// 运行代理服务器
    /// 
    /// # 返回值
    /// 返回Result，如果过程中出现错误则返回错误信息
    pub async fn run(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.proxy.host, self.config.proxy.port);
        let listener = TcpListener::bind(&addr).await?;
        log::info!("🚀 Proxy server running on http://{}", addr);

        loop {
            tokio::select! {
                // 接受新的连接
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            log::info!("📥 New connection from {addr}");
                            
                            // 克隆Arc引用以在异步任务中使用
                            let config = Arc::clone(&self.config);
                            let cert_manager = Arc::clone(&self.cert_manager);
                            let logger = Arc::clone(&self.logger);
                            
                            // 为每个连接创建异步任务
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, config, cert_manager, logger).await {
                                    log::error!("❌ Connection error: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            log::error!("❌ Failed to accept connection: {e}");
                        }
                    }
                }
                // 监听关闭信号
                _ = signal::ctrl_c() => {
                    log::info!("🛑 Shutting down proxy server...");
                    break;
                }
            }
        }

        Ok(())
    }
}