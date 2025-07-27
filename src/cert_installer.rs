use anyhow::Result;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::fs;
use std::env;
use std::path::PathBuf;

/// 证书安装器，负责将生成的CA证书安装到系统信任存储
pub struct CertInstaller {
    ca_cert_path: String,
    cert_name: String,
}

impl CertInstaller {
    /// 创建新的证书安装器
    pub fn new(ca_cert_path: &str, cert_name: &str) -> Self {
        Self {
            ca_cert_path: ca_cert_path.to_string(),
            cert_name: cert_name.to_string(),
        }
    }

    /// 自动安装证书到系统信任存储
    pub async fn install_certificate(&self) -> Result<bool> {
        log::info!("Installing CA certificate to system trust store...");
        
        #[cfg(target_os = "macos")]
        return self.install_macos().await;
        
        #[cfg(target_os = "linux")]
        return self.install_linux().await;
        
        #[cfg(target_os = "windows")]
        return self.install_windows().await;
        
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        log::warn!("Certificate installation not supported on this platform");
        Ok(false)
    }
}

    /// 自动配置curl环境
    pub async fn configure_curl_environment(&self, proxy_host: &str, proxy_port: u16) -> Result<()> {
        log::info!("Configuring curl environment...");
        
        #[cfg(target_os = "macos")]
        return self.configure_curl_macos(proxy_host, proxy_port).await;
        
        #[cfg(target_os = "linux")]
        return self.configure_curl_linux(proxy_host, proxy_port).await;
        
        #[cfg(target_os = "windows")]
        return self.configure_curl_windows(proxy_host, proxy_port).await;
        
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            log::warn!("Curl configuration not supported on this platform");
            Ok(())
        }
    }

    #[cfg(target_os = "macos")]
    async fn configure_curl_macos(&self, proxy_host: &str, proxy_port: u16) -> Result<()> {
        // 创建.curlrc文件
        let home_dir = env::var("HOME")?;
        let curlrc_path = PathBuf::from(home_dir).join(".curlrc");
        
        let ca_cert_abs_path = std::fs::canonicalize(&self.ca_cert_path)
            .unwrap_or_else(|_| PathBuf::from(&self.ca_cert_path))
            .to_string_lossy()
            .to_string();
            
        let curl_config = format!(
            "# Study Proxy Auto Configuration\nproxy = {}:{}\ncacert = {}\n",
            proxy_host, proxy_port, ca_cert_abs_path
        );
        
        fs::write(&curlrc_path, curl_config)?;
        log::info!("Created curl configuration: {:?}", curlrc_path);
        
        // 配置环境变量
        self.configure_shell_env(proxy_host, proxy_port).await?;
        
        Ok(())
    }

    #[cfg(target_os = "linux")]
    async fn configure_curl_linux(&self, proxy_host: &str, proxy_port: u16) -> Result<()> {
        // 创建.curlrc文件
        let home_dir = env::var("HOME")?;
        let curlrc_path = PathBuf::from(home_dir).join(".curlrc");
        
        let curl_config = format!(
            "# Study Proxy Auto Configuration\nproxy = {}:{}\ncacert = {}\n",
            proxy_host, proxy_port, self.ca_cert_path
        );
        
        fs::write(&curlrc_path, curl_config)?;
        log::info!("Created curl configuration: {:?}", curlrc_path);
        
        // 配置环境变量
        self.configure_shell_env(proxy_host, proxy_port).await?;
        
        Ok(())
    }

    #[cfg(target_os = "windows")]
    async fn configure_curl_windows(&self, _proxy_host: &str, _proxy_port: u16) -> Result<()> {
        log::info!("Windows curl configuration not implemented");
        Ok(())
    }

    async fn configure_shell_env(&self, proxy_host: &str, proxy_port: u16) -> Result<()> {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let home_dir = env::var("HOME")?;
        
        let shell_config = if shell.contains("zsh") {
            PathBuf::from(&home_dir).join(".zshrc")
        } else if shell.contains("bash") {
            PathBuf::from(&home_dir).join(".bashrc")
        } else {
            PathBuf::from(&home_dir).join(".profile")
        };
        
        let ca_cert_abs_path = std::fs::canonicalize(&self.ca_cert_path)
            .unwrap_or_else(|_| PathBuf::from(&self.ca_cert_path))
            .to_string_lossy()
            .to_string();
            
        let env_config = format!(
            "\n# Study Proxy Auto Configuration\nexport HTTPS_PROXY=http://{}:{}\nexport HTTP_PROXY=http://{}:{}\nexport CURL_CA_BUNDLE={}\n",
            proxy_host, proxy_port, proxy_host, proxy_port, ca_cert_abs_path
        );
        
        // 检查是否已存在配置
        if let Ok(existing_content) = fs::read_to_string(&shell_config) {
            if existing_content.contains("# Study Proxy Auto Configuration") {
                log::info!("Shell configuration already exists, skipping");
                return Ok(());
            }
            
            // 追加配置到shell配置文件
            let mut new_content = existing_content;
            if !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push_str(&env_config);
            fs::write(&shell_config, new_content)?;
            log::info!("Appended shell configuration: {:?}", shell_config);
        } else {
            // 文件不存在，创建新文件
            fs::write(&shell_config, env_config)?;
            log::info!("Created new shell configuration: {:?}", shell_config);
        }
        
        Ok(())
    }

    /// 清理curl环境配置
    pub async fn cleanup_curl_environment(&self) -> Result<()> {
        log::info!("Starting cleanup of curl environment variables...");
        
        let home_dir = env::var("HOME")?;
        
        // 删除.curlrc文件
        let curlrc_path = PathBuf::from(&home_dir).join(".curlrc");
        if curlrc_path.exists() {
            fs::remove_file(&curlrc_path)?;
            log::info!("✓ Removed .curlrc configuration file: {:?}", curlrc_path);
        } else {
            log::info!("✓ .curlrc file not found, nothing to clean");
        }
        
        // 清理shell配置文件中的环境变量
        self.cleanup_shell_env().await?;
        
        log::info!("✓ Environment cleanup completed - HTTP_PROXY, HTTPS_PROXY, and CURL_CA_BUNDLE variables have been removed");
        
        Ok(())
    }

    async fn cleanup_shell_env(&self) -> Result<()> {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let home_dir = env::var("HOME")?;
        
        let shell_config = if shell.contains("zsh") {
            PathBuf::from(&home_dir).join(".zshrc")
        } else if shell.contains("bash") {
            PathBuf::from(&home_dir).join(".bashrc")
        } else {
            PathBuf::from(&home_dir).join(".profile")
        };
        
        log::info!("Starting cleanup for shell config: {:?}", shell_config);
        
        if let Ok(content) = fs::read_to_string(&shell_config) {
            log::debug!("Original content:\n{}", content);
            
            let lines: Vec<&str> = content.lines().collect();
            let mut result = Vec::new();
            let mut removed_lines = 0;
            let mut in_proxy_block = false;
            
            for line in lines {
                let trimmed = line.trim();
                
                if trimmed == "# Study Proxy Auto Configuration" {
                    log::debug!("Found Study Proxy config block start, removing...");
                    removed_lines += 1;
                    in_proxy_block = true;
                    continue;
                }
                
                if in_proxy_block {
                    if trimmed.starts_with("export HTTPS_PROXY=") ||
                       trimmed.starts_with("export HTTP_PROXY=") ||
                       trimmed.starts_with("export CURL_CA_BUNDLE=") {
                        log::debug!("Removing proxy line: {}", trimmed);
                        removed_lines += 1;
                        continue;
                    } else if trimmed.is_empty() || trimmed.starts_with('#') {
                        // 跳过空行和注释，继续在同一block中
                        log::debug!("Skipping empty/comment line in proxy block: {}", trimmed);
                        removed_lines += 1;
                        continue;
                    } else {
                        // 遇到非代理相关行，退出block模式
                        in_proxy_block = false;
                    }
                }
                
                // 检查单独的代理行（不在block中）
                if !trimmed.starts_with("export HTTPS_PROXY=") &&
                   !trimmed.starts_with("export HTTP_PROXY=") &&
                   !trimmed.starts_with("export CURL_CA_BUNDLE=") {
                    result.push(line);
                } else {
                    log::debug!("Removing standalone proxy line: {}", trimmed);
                    removed_lines += 1;
                }
            }
            
            let new_content = result.join("\n");
            log::debug!("New content:\n{}", new_content);
            
            fs::write(&shell_config, new_content)?;
            log::info!("✓ Successfully removed {} lines from {:?}", removed_lines, shell_config);
        } else {
            log::warn!("Could not read shell config file: {:?}", shell_config);
        }
        
        Ok(())
    }

    /// 卸载证书从系统信任存储
    pub async fn uninstall_certificate(&self) -> Result<bool> {
        log::info!("Removing CA certificate from system trust store...");
        
        #[cfg(target_os = "macos")]
        return self.uninstall_macos().await;
        
        #[cfg(target_os = "linux")]
        return self.uninstall_linux().await;
        
        #[cfg(target_os = "windows")]
        return self.uninstall_windows().await;
        
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            log::warn!("Certificate removal not supported on this platform");
            Ok(false)
        }
    }

    #[cfg(target_os = "macos")]
    async fn install_macos(&self) -> Result<bool> {
        let cert_path = &self.ca_cert_path;
        
        // 检查证书是否已安装
        if self.is_macos_cert_installed().await? {
            log::info!("CA certificate already installed on macOS");
            return Ok(true);
        }

        // 安装证书到系统钥匙串
        let output = Command::new("sudo")
            .args(&[
                "security",
                "add-trusted-cert",
                "-d",
                "-r", "trustRoot",
                "-k", "/Library/Keychains/System.keychain",
                cert_path,
            ])
            .output()?;

        if output.status.success() {
            log::info!("CA certificate successfully installed to macOS system keychain");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            log::error!("Failed to install certificate: {}", error);
            Ok(false)
        }
    }

    #[cfg(target_os = "macos")]
    async fn uninstall_macos(&self) -> Result<bool> {
        let _cert_path = &self.ca_cert_path;
        
        // 从系统钥匙串中删除证书
        let output = Command::new("sudo")
            .args(&[
                "security",
                "delete-certificate",
                "-c", &self.cert_name,
                "/Library/Keychains/System.keychain",
            ])
            .output()?;

        if output.status.success() {
            log::info!("CA certificate successfully removed from macOS system keychain");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            log::warn!("Failed to remove certificate (may not exist): {}", error);
            Ok(false)
        }
    }

    #[cfg(target_os = "macos")]
    async fn is_macos_cert_installed(&self) -> Result<bool> {
        let output = Command::new("security")
            .args(&[
                "find-certificate",
                "-c", &self.cert_name,
                "/Library/Keychains/System.keychain",
            ])
            .output()?;

        Ok(output.status.success())
    }

    #[cfg(target_os = "linux")]
    async fn install_linux(&self) -> Result<bool> {
        let cert_path = &self.ca_cert_path;
        let cert_name = &self.cert_name;
        let target_path = format!("/usr/local/share/ca-certificates/{}.crt", cert_name);

        // 检查证书是否已安装
        if Path::new(&target_path).exists() {
            log::info!("CA certificate already installed on Linux");
            return Ok(true);
        }

        // 复制证书到系统证书目录
        let copy_output = Command::new("sudo")
            .args(&["cp", cert_path, &target_path])
            .output()?;

        if !copy_output.status.success() {
            let error = String::from_utf8_lossy(&copy_output.stderr);
            log::error!("Failed to copy certificate: {}", error);
            return Ok(false);
        }

        // 更新证书存储
        let update_output = Command::new("sudo")
            .args(&["update-ca-certificates"])
            .output()?;

        if update_output.status.success() {
            log::info!("CA certificate successfully installed to Linux system trust store");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&update_output.stderr);
            log::error!("Failed to update certificates: {}", error);
            Ok(false)
        }
    }

    #[cfg(target_os = "linux")]
    async fn uninstall_linux(&self) -> Result<bool> {
        let cert_name = &self.cert_name;
        let cert_path = format!("/usr/local/share/ca-certificates/{}.crt", cert_name);

        if !Path::new(&cert_path).exists() {
            log::info!("CA certificate not found on Linux");
            return Ok(true);
        }

        // 删除证书文件
        let remove_output = Command::new("sudo")
            .args(&["rm", &cert_path])
            .output()?;

        if !remove_output.status.success() {
            let error = String::from_utf8_lossy(&remove_output.stderr);
            log::error!("Failed to remove certificate: {}", error);
            return Ok(false);
        }

        // 更新证书存储
        let update_output = Command::new("sudo")
            .args(&["update-ca-certificates", "--fresh"])
            .output()?;

        if update_output.status.success() {
            log::info!("CA certificate successfully removed from Linux system trust store");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&update_output.stderr);
            log::warn!("Failed to update certificates: {}", error);
            Ok(false)
        }
    }

    #[cfg(target_os = "windows")]
    async fn install_windows(&self) -> Result<bool> {
        let cert_path = &self.ca_cert_path;

        // 安装证书到受信任的根证书颁发机构
        let output = Command::new("certutil")
            .args(&[
                "-addstore",
                "-f",
                "Root",
                cert_path,
            ])
            .output()?;

        if output.status.success() {
            log::info!("CA certificate successfully installed to Windows certificate store");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            log::error!("Failed to install certificate: {}", error);
            Ok(false)
        }
    }

    #[cfg(target_os = "windows")]
    async fn uninstall_windows(&self) -> Result<bool> {
        // 从证书存储中删除证书（通过颁发者名称）
        let output = Command::new("certutil")
            .args(&[
                "-delstore",
                "Root",
                &self.cert_name,
            ])
            .output()?;

        if output.status.success() {
            log::info!("CA certificate successfully removed from Windows certificate store");
            Ok(true)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            log::warn!("Failed to remove certificate: {}", error);
            Ok(false)
        }
    }
}