use anyhow::Result;
use rustls::{Certificate, PrivateKey};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use rcgen::{date_time_ymd, KeyPair, PKCS_ECDSA_P256_SHA256};

/// 证书管理器，负责CA证书的生成、加载和站点证书的签发
/// 
/// 该结构体实现了证书的持久化存储，当证书文件存在时会复用现有证书，
/// 不存在时则会生成新的证书并保存到文件系统中。
pub struct CertManager {
    /// CA证书
    ca_cert: Certificate,
    /// CA私钥
    ca_key: PrivateKey,
}

impl CertManager {
    /// 创建新的证书管理器
    /// 
    /// # 参数
    /// * `ca_cert_path` - CA证书文件路径
    /// * `ca_key_path` - CA私钥文件路径
    /// 
    /// # 返回值
    /// 返回Result包装的CertManager实例，如果过程中出现错误则返回错误信息
    pub fn new(ca_cert_path: &str, ca_key_path: &str) -> Result<Self> {
        let ca_cert_path = Path::new(ca_cert_path);
        let ca_key_path = Path::new(ca_key_path);
        
        // 尝试加载已存在的证书和私钥
        match Self::load_existing_certificates(ca_cert_path, ca_key_path) {
            Some(cert_manager) => {
                log::info!("Successfully loaded existing CA certificate and key");
                Ok(cert_manager)
            },
            None => {
                // 生成新的CA证书和私钥
                log::info!("Generating new CA certificate...");
                let (ca_cert, ca_key) = Self::generate_ca_cert()?;
                
                // 保存证书和私钥到文件
                Self::save_certificates(ca_cert_path, ca_key_path, &ca_cert, &ca_key)?;
                log::info!("Saved new CA certificate and key to files");
                
                Ok(Self { ca_cert, ca_key })
            }
        }
    }

    /// 加载已存在的证书和私钥文件
    /// 
    /// # 参数
    /// * `ca_cert_path` - CA证书文件路径
    /// * `ca_key_path` - CA私钥文件路径
    /// 
    /// # 返回值
    /// 如果证书和私钥文件都存在且能成功加载，则返回Some(CertManager)，
    /// 否则返回None
    fn load_existing_certificates(
        ca_cert_path: &Path,
        ca_key_path: &Path,
    ) -> Option<Self> {
        // 检查证书和私钥文件是否都存在
        if !(ca_cert_path.exists() && ca_key_path.exists()) {
            log::info!("CA certificate or key file not found, will generate new ones");
            return None;
        }

        log::info!("Loading existing CA certificate and key...");
        let cert_file = File::open(ca_cert_path).ok()?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs = rustls_pemfile::certs(&mut cert_reader).ok()?;

        let key_file = File::open(ca_key_path).ok()?;
        let mut key_reader = BufReader::new(key_file);
        let keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader).ok()?;

        // 确保证书和私钥都成功加载
        match (certs.first(), keys.first()) {
            (Some(cert_data), Some(key_data)) => {
                let ca_cert = Certificate(cert_data.clone());
                let ca_key = PrivateKey(key_data.clone());
                Some(Self { ca_cert, ca_key })
            },
            _ => {
                log::warn!("Failed to load existing certificate or key data");
                None
            }
        }
    }

    /// 保存证书和私钥到文件
    /// 
    /// # 参数
    /// * `ca_cert_path` - CA证书文件保存路径
    /// * `ca_key_path` - CA私钥文件保存路径
    /// * `ca_cert` - CA证书
    /// * `ca_key` - CA私钥
    /// 
    /// # 返回值
    /// 如果保存成功返回Ok(())，否则返回错误信息
    fn save_certificates(
        ca_cert_path: &Path,
        ca_key_path: &Path,
        ca_cert: &Certificate,
        ca_key: &PrivateKey,
    ) -> Result<()> {
        // 确保证书目录存在
        if let Some(parent) = ca_cert_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // 保存证书
        let cert_file = File::create(ca_cert_path)?;
        let mut cert_writer = BufWriter::new(cert_file);
        let cert_pem = pem::Pem::new("CERTIFICATE", ca_cert.0.clone());
        writeln!(cert_writer, "{}", pem::encode(&cert_pem))?;
        
        // 保存私钥
        let key_file = File::create(ca_key_path)?;
        let mut key_writer = BufWriter::new(key_file);
        let key_pem = pem::Pem::new("PRIVATE KEY", ca_key.0.clone());
        writeln!(key_writer, "{}", pem::encode(&key_pem))?;
        
        Ok(())
    }

    /// 生成CA证书和私钥
    /// 
    /// # 返回值
    /// 返回Result包装的(Certificate, PrivateKey)元组，如果过程中出现错误则返回错误信息
    fn generate_ca_cert() -> Result<(Certificate, PrivateKey)> {
        use rcgen::*;
        
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Study Proxy CA");
        params.distinguished_name = dn;
        
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        
        // 设置有效期为当前时间开始的合理范围
        params.not_before = date_time_ymd(2024, 1, 1);
        params.not_after = date_time_ymd(2034, 12, 31);
        
        let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256)?;
        params.alg = &PKCS_ECDSA_P256_SHA256;
        params.key_pair = Some(key_pair);
        
        let cert = Certificate::from_params(params)?;
        let cert_pem = cert.serialize_pem()?;
        let key_pem = cert.serialize_private_key_pem();
        
        // 解析PEM格式的证书和私钥
        let cert_pem_obj = pem::parse(cert_pem.as_bytes())?;
        let private_key_pem_obj = pem::parse(key_pem.as_bytes())?;
        
        Ok((
            Certificate(cert_pem_obj.into_contents()),
            PrivateKey(private_key_pem_obj.into_contents()),
        ))
    }

    /// 为指定域名生成站点证书
    /// 
    /// # 参数
    /// * `domain` - 需要生成证书的域名
    /// 
    /// # 返回值
    /// 返回Result包装的(Vec<u8>, Vec<u8>)元组，分别表示证书PEM数据和私钥PEM数据，
    /// 如果过程中出现错误则返回错误信息
    pub fn generate_site_cert(&self, domain: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        use rcgen::*;
        
        let mut params = CertificateParams::default();
        
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, domain);
        params.distinguished_name = dn;
        
        params.subject_alt_names = vec![SanType::DnsName(domain.to_string())];
        params.alg = &PKCS_ECDSA_P256_SHA256;
        
        // 设置站点证书有效期
        params.not_before = date_time_ymd(2024, 1, 1);
        params.not_after = date_time_ymd(2026, 12, 31);
        
        // 添加关键扩展
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        
        let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256)?;
        params.key_pair = Some(key_pair);
        
        let cert = Certificate::from_params(params)?;
        
        // 使用CA证书签署站点证书
        let cert_pem = cert.serialize_pem_with_signer(&self.load_signer()?)?;
        let key_pem = cert.serialize_private_key_pem();
        
        Ok((cert_pem.into_bytes(), key_pem.into_bytes()))
    }
    
    /// 从已加载的CA私钥加载签名器
    /// 
    /// # 返回值
    /// 返回Result包装的rcgen::Certificate，如果过程中出现错误则返回错误信息
    fn load_signer(&self) -> Result<rcgen::Certificate> {
        use rcgen::*;
        use anyhow::Context;
        
        // 创建一个临时的CertificateParams来重建CA证书
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Study Proxy CA");
        params.distinguished_name = dn;
        
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        
        // 使用CA私钥重建KeyPair
        let key_pair = KeyPair::from_der(&self.ca_key.0)
            .context("Failed to create KeyPair from DER")?;
        params.alg = key_pair.algorithm();
        params.key_pair = Some(key_pair);
        
        // 设置有效期
        params.not_before = date_time_ymd(2024, 1, 1);
        params.not_after = date_time_ymd(2034, 12, 31);
        
        Ok(Certificate::from_params(params)?)
    }
}