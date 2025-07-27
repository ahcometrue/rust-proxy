use anyhow::Result;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

/// 证书环境管理器 - 负责CA证书的系统级安装和卸载
///
/// 本模块专门处理CA证书在操作系统中的信任存储管理：
/// 1. 跨平台证书安装（macOS、Linux、Windows）
/// 2. 证书信任状态的检测和管理
/// 3. 证书的安全卸载和清理
/// 4. 系统权限检查和错误处理
pub struct CertManager {
    ca_cert_path: String,
    cert_name: String,
}

impl CertManager {
    /// 创建新的证书环境管理器
    pub fn new(ca_cert_path: &str, cert_name: &str) -> Self {
        Self {
            ca_cert_path: ca_cert_path.to_string(),
            cert_name: cert_name.to_string(),
        }
    }

    /// 自动安装CA证书到系统信任存储
    ///
    /// 根据当前操作系统平台，自动执行以下操作：
    /// - macOS: 安装到系统钥匙串并设置为信任根证书
    /// - Linux: 复制到系统证书目录并更新证书存储
    /// - Windows: 安装到受信任的根证书颁发机构存储
    ///
    /// 返回：成功返回true，失败返回false
    pub async fn install_ca_certificate(&self) -> Result<bool> {
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

    /// 从系统信任存储中卸载CA证书
    ///
    /// 安全地从操作系统中移除CA证书：
    /// - 检测证书是否存在
    /// - 使用系统原生工具进行卸载
    /// - 更新系统证书缓存
    ///
    /// 返回：成功返回true，失败返回false
    pub async fn uninstall_ca_certificate(&self) -> Result<bool> {
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
    /// macOS平台证书安装实现
    ///
    /// 使用security命令将CA证书安装到系统钥匙串：
    /// - 需要管理员权限（sudo）
    /// - 自动设置为信任根证书
    /// - 安装前检查是否已存在同名证书
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
    /// macOS平台证书卸载实现
    ///
    /// 使用security命令从系统钥匙串中移除CA证书：
    /// - 需要管理员权限（sudo）
    /// - 通过证书名称进行识别和删除
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
    /// 检查macOS系统钥匙串中是否已安装指定证书
    ///
    /// 使用security命令查询证书是否存在：
    /// - 通过证书名称进行查找
    /// - 返回证书是否已安装的布尔值
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
    /// Linux平台证书安装实现
    ///
    /// 将CA证书安装到Linux系统信任存储：
    /// - 复制证书文件到/usr/local/share/ca-certificates/
    /// - 使用update-ca-certificates命令更新系统证书存储
    /// - 需要sudo权限执行文件复制和证书更新
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
    /// Linux平台证书卸载实现
    ///
    /// 从Linux系统信任存储中移除CA证书：
    /// - 删除/usr/local/share/ca-certificates/中的证书文件
    /// - 使用update-ca-certificates --fresh重新生成证书存储
    /// - 需要sudo权限执行文件删除和证书更新
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
    /// Windows平台证书安装实现
    ///
    /// 将CA证书安装到Windows证书存储：
    /// - 使用certutil命令安装到受信任的根证书颁发机构
    /// - 需要管理员权限运行
    /// - 自动处理证书格式和信任设置
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
    /// Windows平台证书卸载实现
    ///
    /// 从Windows证书存储中移除CA证书：
    /// - 使用certutil命令从受信任的根证书颁发机构中删除
    /// - 通过证书名称进行识别和删除
    /// - 需要管理员权限运行
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