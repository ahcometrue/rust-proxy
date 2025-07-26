use anyhow::Result;
use rustls::{Certificate, PrivateKey};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use rcgen::{date_time_ymd, KeyPair, PKCS_ECDSA_P256_SHA256};

pub struct CertManager {
    ca_cert: Certificate,
    ca_key: PrivateKey,
}

impl CertManager {
    pub fn new(ca_cert_path: &str, ca_key_path: &str) -> Result<Self> {
        let ca_cert_path = Path::new(ca_cert_path);
        let ca_key_path = Path::new(ca_key_path);
        
        // 如果证书和私钥文件都存在，则加载它们
        if ca_cert_path.exists() && ca_key_path.exists() {
            log::info!("Loading existing CA certificate and key...");
            let cert_file = File::open(ca_cert_path)?;
            let mut cert_reader = BufReader::new(cert_file);
            let certs = rustls_pemfile::certs(&mut cert_reader)?;
            
            let key_file = File::open(ca_key_path)?;
            let mut key_reader = BufReader::new(key_file);
            let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)?;
            
            if !certs.is_empty() && !keys.is_empty() {
                let ca_cert = Certificate(certs[0].clone());
                let ca_key = PrivateKey(keys.remove(0));
                
                log::info!("Successfully loaded existing CA certificate and key");
                return Ok(Self { ca_cert, ca_key });
            }
        }
        
        // 否则生成新的CA证书和私钥（使用rcgen）
        log::info!("Generating new CA certificate...");
        let (ca_cert, ca_key) = Self::generate_ca_cert()?;
        
        // 保存证书和私钥到文件
        fs::create_dir_all(ca_cert_path.parent().unwrap())?;
        
        let cert_file = File::create(ca_cert_path)?;
        let mut cert_writer = BufWriter::new(cert_file);
        let cert_pem = pem::Pem::new("CERTIFICATE", ca_cert.0.clone());
        writeln!(cert_writer, "{}", pem::encode(&cert_pem))?;
        
        let key_file = File::create(ca_key_path)?;
        let mut key_writer = BufWriter::new(key_file);
        let key_pem = pem::Pem::new("PRIVATE KEY", ca_key.0.clone());
        writeln!(key_writer, "{}", pem::encode(&key_pem))?;
        
        log::info!("Saved new CA certificate and key to files");
        
        Ok(Self { ca_cert, ca_key })
    }

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