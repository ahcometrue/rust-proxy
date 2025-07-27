use anyhow::Result;
use std::fs;
use std::env;
use std::path::PathBuf;

/// curl环境管理器 - 负责配置和清理curl代理环境
/// 
/// 本模块专门处理curl工具的代理配置，包括：
/// 1. .curlrc配置文件的创建和管理
/// 2. shell环境变量配置（HTTPS_PROXY、HTTP_PROXY、CURL_CA_BUNDLE）
/// 3. 跨平台支持（macOS、Linux、Windows）
/// 4. 清理功能，确保程序退出时恢复原始环境
pub struct CurlManager {
    ca_cert_path: String,
}

impl CurlManager {
    /// 创建新的curl环境管理器
    pub fn new(ca_cert_path: &str) -> Self {
        Self {
            ca_cert_path: ca_cert_path.to_string(),
        }
    }

    /// 自动配置curl代理环境
    /// 
    /// 根据当前平台配置curl的代理设置，包括：
    /// - 创建.curlrc文件配置代理服务器和CA证书
    /// - 配置shell环境变量支持代理
    /// - 确保配置不会重复添加
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
    /// macOS平台curl环境配置实现
    ///
    /// 为macOS系统配置curl代理环境：
    /// - 创建用户主目录下的.curlrc文件
    /// - 配置代理服务器地址和端口
    /// - 设置CA证书路径（使用绝对路径确保可靠性）
    /// - 配置shell环境变量支持代理
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
    /// Linux平台curl环境配置实现
    ///
    /// 为Linux系统配置curl代理环境：
    /// - 创建用户主目录下的.curlrc文件
    /// - 配置代理服务器地址和端口
    /// - 设置CA证书路径
    /// - 配置shell环境变量支持代理
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
    /// Windows平台curl环境配置实现
    ///
    /// Windows系统的curl环境配置占位符
    /// - 当前版本暂未实现Windows平台的curl配置
    /// - 保留接口以便后续扩展支持
    async fn configure_curl_windows(&self, _proxy_host: &str, _proxy_port: u16) -> Result<()> {
        log::info!("Windows curl configuration not implemented");
        Ok(())
    }

    /// 配置shell环境变量以支持curl代理
    ///
    /// 自动检测用户使用的shell类型，并配置相应的环境变量：
    /// - 支持zsh、bash等主流shell
    /// - 配置HTTPS_PROXY、HTTP_PROXY和CURL_CA_BUNDLE变量
    /// - 智能检测避免重复配置
    /// - 自动处理文件末尾换行符
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

    /// 清理curl代理环境配置
    /// 
    /// 彻底清理由本管理器创建的所有curl相关配置：
    /// - 删除.curlrc配置文件
    /// - 清理shell配置文件中的代理环境变量
    /// - 保持用户原有配置不受影响
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

    /// 清理shell配置文件中的代理环境变量
    ///
    /// 精确识别并清理由本管理器添加的代理配置：
    /// - 使用"# Study Proxy Auto Configuration"作为标识符
    /// - 智能处理配置块和单独代理行
    /// - 保留用户原有的其他配置
    /// - 详细记录清理过程便于调试
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
}