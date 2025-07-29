mod config;
mod cert;
mod proxy {
    pub mod proxy_server;
    pub mod handler;
    pub mod response;
    pub mod request;
}
mod domain_logger;
mod system_proxy;
mod cert_manager;
mod curl_manager;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use system_proxy::{SystemProxyManager, ProxyConfig};
use cert_manager::CertManager as CertEnvManager;
use curl_manager::CurlManager;
use proxy::proxy_server::ProxyServer;

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
    if config.system_proxy.enabled {
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

    // 创建证书管理器
    let cert_manager = Arc::new(cert::CertManager::new(
        &config.certificates.ca_cert,
        &config.certificates.ca_key,
        &config.certificates.name,
    )?);
    
    // 创建日志记录器
    let logger = domain_logger::DomainLogger::new(Arc::new(config.clone()));
    
    // 先创建代理服务器（会生成证书）
    log::info!("Starting proxy server...");
    let server = ProxyServer::new(
        Arc::new(config.clone()),
        Arc::clone(&cert_manager),
        Arc::clone(&logger),
    );
    
    // 自动安装证书到系统信任存储（确保证书已生成）
    if config.certificates.auto_install {
        let cert_manager = CertEnvManager::new(&config.certificates.ca_cert, &config.certificates.name);
        if let Err(e) = cert_manager.install_ca_certificate().await {
            log::warn!("Failed to install CA certificate: {}", e);
        }
        
        // 自动配置curl环境
        if config.certificates.configure_curl {
            let curl_manager = CurlManager::new(&config.certificates.ca_cert);
            if let Err(e) = curl_manager.configure_curl_environment(&config.proxy.host, config.proxy.port).await {
                log::warn!("Failed to configure curl environment: {}", e);
            } else {
                log::info!("Curl environment configured successfully");
            }
        }
    }
    
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
    if config.system_proxy.enabled {
        log::info!("Cleaning up system proxy settings...");
        if let Err(e) = proxy_manager.unset_proxy().await {
            log::warn!("Failed to unset system proxy: {}", e);
        } else {
            log::info!("System proxy settings restored");
        }
    }

    // 清理curl环境配置（当configure_curl为true时）
    if config.certificates.configure_curl {
        let curl_manager = CurlManager::new(&config.certificates.ca_cert);
        log::info!("Cleaning up curl environment...");
        if let Err(e) = curl_manager.cleanup_curl_environment().await {
            log::warn!("Failed to cleanup curl environment: {}", e);
        } else {
            log::info!("Curl environment cleaned up successfully");
        }
    }
    
    // 卸载系统信任存储中的证书（当auto_uninstall为true时）
    if config.certificates.auto_uninstall {
        let cert_manager = CertEnvManager::new(&config.certificates.ca_cert, &config.certificates.name);
        if let Err(e) = cert_manager.uninstall_ca_certificate().await {
            log::warn!("Failed to uninstall CA certificate: {}", e);
        }
    }
    
    Ok(())
}
