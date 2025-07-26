mod config;
mod cert;
mod proxy;
mod domain_logger;

use anyhow::Result;
use clap::Parser;

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
    
    log::info!("Starting proxy server...");
    let server = proxy::ProxyServer::new(config)?;
    server.run().await?;
    
    Ok(())
}
