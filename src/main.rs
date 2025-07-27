mod config;
mod cert;
mod proxy;
mod domain_logger;
mod system_proxy;

use anyhow::Result;
use clap::Parser;
use system_proxy::{SystemProxyManager, ProxyConfig};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "config.json")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    
    log::info!("Loading configuration from: {}", cli.config);
    let config = config::Config::from_file(&cli.config)?;
    
    // 创建系统代理管理器
    let proxy_manager = SystemProxyManager::new()?;
    
    // 设置系统代理
    if config.system_proxy.enabled && config.system_proxy.auto_configure {
        let proxy_config = ProxyConfig {
            host: config.proxy.host.clone(),
            port: config.proxy.port,
            enabled: true,
        };
        
        if let Err(e) = proxy_manager.set_proxy(&proxy_config).await {
            log::warn!("Failed to set system proxy: {}", e);
        } else {
            log::info!("System proxy configured successfully");
        }
    }
    
    log::info!("Starting proxy server...");
    let server = proxy::ProxyServer::new(config.clone())?;
    
    // 使用tokio::select!来同时监听信号和服务器运行
    tokio::select! {
        server_result = server.run() => {
            match server_result {
                Ok(_) => {
                    log::info!("Proxy server stopped normally");
                }
                Err(e) => {
                    log::error!("Proxy server error: {}", e);
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            log::info!("Received interrupt signal, shutting down...");
        }
    }
    
    // 清理系统代理
    if config.system_proxy.enabled && config.system_proxy.auto_configure {
        log::info!("Cleaning up system proxy settings...");
        if let Err(e) = proxy_manager.unset_proxy().await {
            log::warn!("Failed to unset system proxy: {}", e);
        } else {
            log::info!("System proxy settings restored");
        }
    }
    
    Ok(())
}
