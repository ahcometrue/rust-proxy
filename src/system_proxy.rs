use anyhow::{Result, Context};
use std::process::Command;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SystemProxyManager {
    original_settings: Arc<Mutex<Option<OriginalProxySettings>>>,
    platform: Platform,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct OriginalProxySettings {
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    ftp_proxy: Option<String>,
    socks_proxy: Option<String>,
    auto_proxy: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
enum Platform {
    Windows,
    MacOS,
    Linux,
    Unknown,
}

impl SystemProxyManager {
    pub fn new() -> Result<Self> {
        let platform = detect_platform();
        
        Ok(Self {
            original_settings: Arc::new(Mutex::new(None)),
            platform,
        })
    }

    pub async fn set_proxy(&self, config: &ProxyConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }

        let proxy_url = format!("http://{}:{}", config.host, config.port);
        
        // 保存原始设置
        let original = self.get_current_proxy_settings().await?;
        {
            let mut settings = self.original_settings.lock().await;
            *settings = Some(original);
        }

        match self.platform {
            Platform::Windows => self.set_windows_proxy(&proxy_url).await,
            Platform::MacOS => self.set_macos_proxy(&proxy_url).await,
            Platform::Linux => self.set_linux_proxy(&proxy_url).await,
            Platform::Unknown => Err(anyhow::anyhow!("Unsupported platform")),
        }
    }

    pub async fn unset_proxy(&self) -> Result<()> {
        let settings = {
            let mut settings = self.original_settings.lock().await;
            settings.take()
        };

        if let Some(original) = settings {
            match self.platform {
                Platform::Windows => self.unset_windows_proxy(original).await,
                Platform::MacOS => self.unset_macos_proxy(original).await,
                Platform::Linux => self.unset_linux_proxy(original).await,
                Platform::Unknown => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    async fn get_current_proxy_settings(&self) -> Result<OriginalProxySettings> {
        match self.platform {
            Platform::MacOS => self.get_macos_proxy_settings().await,
            Platform::Windows => self.get_windows_proxy_settings().await,
            Platform::Linux => self.get_linux_proxy_settings().await,
            Platform::Unknown => Ok(OriginalProxySettings::default()),
        }
    }

    // macOS 实现
    async fn set_macos_proxy(&self, _proxy_url: &str) -> Result<()> {
        let output = Command::new("networksetup")
            .args(&["-setwebproxy", "Wi-Fi", "127.0.0.1", "8888"])
            .output()
            .context("Failed to set macOS HTTP proxy")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to set macOS HTTP proxy: {}", 
                String::from_utf8_lossy(&output.stderr)));
        }

        let output = Command::new("networksetup")
            .args(&["-setsecurewebproxy", "Wi-Fi", "127.0.0.1", "8888"])
            .output()
            .context("Failed to set macOS HTTPS proxy")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to set macOS HTTPS proxy: {}", 
                String::from_utf8_lossy(&output.stderr)));
        }

        log::info!("macOS proxy settings updated for Wi-Fi");
        Ok(())
    }

    async fn unset_macos_proxy(&self, _original: OriginalProxySettings) -> Result<()> {
        let output = Command::new("networksetup")
            .args(&["-setwebproxystate", "Wi-Fi", "off"])
            .output()
            .context("Failed to disable macOS HTTP proxy")?;

        if !output.status.success() {
            log::warn!("Failed to disable macOS HTTP proxy: {}", 
                String::from_utf8_lossy(&output.stderr));
        }

        let output = Command::new("networksetup")
            .args(&["-setsecurewebproxystate", "Wi-Fi", "off"])
            .output()
            .context("Failed to disable macOS HTTPS proxy")?;

        if !output.status.success() {
            log::warn!("Failed to disable macOS HTTPS proxy: {}", 
                String::from_utf8_lossy(&output.stderr));
        }

        log::info!("macOS proxy settings restored");
        Ok(())
    }

    async fn get_macos_proxy_settings(&self) -> Result<OriginalProxySettings> {
        let http_output = Command::new("networksetup")
            .args(&["-getwebproxy", "Wi-Fi"])
            .output()?;

        let https_output = Command::new("networksetup")
            .args(&["-getsecurewebproxy", "Wi-Fi"])
            .output()?;

        let http_proxy = parse_macos_proxy_output(&http_output.stdout)?;
        let https_proxy = parse_macos_proxy_output(&https_output.stdout)?;

        Ok(OriginalProxySettings {
            http_proxy,
            https_proxy,
            ftp_proxy: None,
            socks_proxy: None,
            auto_proxy: None,
        })
    }

    // Windows 实现
    async fn set_windows_proxy(&self, proxy_url: &str) -> Result<()> {
        let proxy_server = proxy_url.trim_start_matches("http://");
        let output = Command::new("reg")
            .args(&[
                "add",
                "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
                "/v",
                "ProxyEnable",
                "/t",
                "REG_DWORD",
                "/d",
                "1",
                "/f",
            ])
            .output()
            .context("Failed to enable Windows proxy")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to enable Windows proxy"));
        }

        let output = Command::new("reg")
            .args(&[
                "add",
                "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
                "/v",
                "ProxyServer",
                "/t",
                "REG_SZ",
                "/d",
                proxy_server,
                "/f",
            ])
            .output()
            .context("Failed to set Windows proxy server")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to set Windows proxy server"));
        }

        log::info!("Windows proxy settings updated");
        Ok(())
    }

    async fn unset_windows_proxy(&self, _original: OriginalProxySettings) -> Result<()> {
        let output = Command::new("reg")
            .args(&[
                "add",
                "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
                "/v",
                "ProxyEnable",
                "/t",
                "REG_DWORD",
                "/d",
                "0",
                "/f",
            ])
            .output()
            .context("Failed to disable Windows proxy")?;

        if !output.status.success() {
            log::warn!("Failed to disable Windows proxy");
        }

        log::info!("Windows proxy settings restored");
        Ok(())
    }

    async fn get_windows_proxy_settings(&self) -> Result<OriginalProxySettings> {
        // Windows代理设置读取较复杂，简化处理
        Ok(OriginalProxySettings::default())
    }

    // Linux 实现
    async fn set_linux_proxy(&self, proxy_url: &str) -> Result<()> {
        // 设置环境变量
        env::set_var("http_proxy", proxy_url);
        env::set_var("https_proxy", proxy_url);
        env::set_var("HTTP_PROXY", proxy_url);
        env::set_var("HTTPS_PROXY", proxy_url);

        // 尝试设置GNOME代理
        let _ = Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy",
                "mode",
                "manual",
            ])
            .output();

        let _ = Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy.http",
                "host",
                "127.0.0.1",
            ])
            .output();

        let _ = Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy.http",
                "port",
                "8888",
            ])
            .output();

        log::info!("Linux proxy settings updated");
        Ok(())
    }

    async fn unset_linux_proxy(&self, _original: OriginalProxySettings) -> Result<()> {
        env::remove_var("http_proxy");
        env::remove_var("https_proxy");
        env::remove_var("HTTP_PROXY");
        env::remove_var("HTTPS_PROXY");

        let _ = Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy",
                "mode",
                "none",
            ])
            .output();

        log::info!("Linux proxy settings restored");
        Ok(())
    }

    async fn get_linux_proxy_settings(&self) -> Result<OriginalProxySettings> {
        // 读取环境变量
        let http_proxy = env::var("http_proxy").ok();
        let https_proxy = env::var("https_proxy").ok();

        Ok(OriginalProxySettings {
            http_proxy,
            https_proxy,
            ftp_proxy: env::var("ftp_proxy").ok(),
            socks_proxy: env::var("socks_proxy").ok(),
            auto_proxy: None,
        })
    }
}

impl Default for OriginalProxySettings {
    fn default() -> Self {
        Self {
            http_proxy: None,
            https_proxy: None,
            ftp_proxy: None,
            socks_proxy: None,
            auto_proxy: None,
        }
    }
}

fn detect_platform() -> Platform {
    #[cfg(target_os = "windows")]
    return Platform::Windows;
    
    #[cfg(target_os = "macos")]
    return Platform::MacOS;
    
    #[cfg(target_os = "linux")]
    return Platform::Linux;
    
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    return Platform::Unknown;
}

fn parse_macos_proxy_output(output: &[u8]) -> Result<Option<String>> {
    let output_str = String::from_utf8_lossy(output);
    let lines: Vec<&str> = output_str.lines().collect();
    
    let mut enabled = false;
    let mut server = String::new();
    let mut port = String::new();
    
    for line in lines {
        if line.contains("Enabled: Yes") {
            enabled = true;
        } else if line.starts_with("Server: ") {
            server = line[8..].trim().to_string();
        } else if line.starts_with("Port: ") {
            port = line[6..].trim().to_string();
        }
    }
    
    if enabled && !server.is_empty() && !port.is_empty() {
        Ok(Some(format!("http://{}:{}", server, port)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let platform = detect_platform();
        assert_ne!(platform, Platform::Unknown);
    }

    #[tokio::test]
    async fn test_proxy_manager_creation() {
        let manager = SystemProxyManager::new().unwrap();
        assert_ne!(manager.platform, Platform::Unknown);
    }
}