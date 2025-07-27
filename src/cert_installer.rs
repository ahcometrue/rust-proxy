use anyhow::Result;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::path::Path;

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