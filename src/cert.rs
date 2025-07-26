use anyhow::Result;
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, KeyPair, SanType, PKCS_ECDSA_P256_SHA256};
use std::fs;
use std::path::Path;

pub struct CertManager {
    ca_cert: Certificate,
}

impl CertManager {
    pub fn new(ca_cert_path: &str, ca_key_path: &str) -> Result<Self> {
        log::info!("Generating CA certificate...");
        let ca_cert = Self::generate_ca()?;
        
        fs::create_dir_all(Path::new(ca_cert_path).parent().unwrap())?;
        fs::write(ca_cert_path, ca_cert.serialize_pem()?)?;
        fs::write(ca_key_path, ca_cert.serialize_private_key_pem())?;
        
        Ok(Self { ca_cert })
    }

    fn generate_ca() -> Result<Certificate> {
        let mut params = CertificateParams::default();
        
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Study Proxy CA");
        params.distinguished_name = dn;
        
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        
        // 设置有效期为当前时间开始的合理范围
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2034, 12, 31);
        
        let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256)?;
        params.alg = &PKCS_ECDSA_P256_SHA256;
        params.key_pair = Some(key_pair);
        
        let cert = Certificate::from_params(params)?;
        Ok(cert)
    }

    pub fn generate_site_cert(&self, domain: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut params = CertificateParams::default();
        
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, domain);
        params.distinguished_name = dn;
        
        params.subject_alt_names = vec![SanType::DnsName(domain.to_string())];
        params.alg = &PKCS_ECDSA_P256_SHA256;
        
        // 设置站点证书有效期
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2025, 12, 31);
        
        // 添加关键扩展
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];
        
        let key_pair = KeyPair::generate(&PKCS_ECDSA_P256_SHA256)?;
        params.key_pair = Some(key_pair);
        
        let cert = Certificate::from_params(params)?;
        
        let cert_pem = cert.serialize_pem_with_signer(&self.ca_cert)?;
        let key_pem = cert.serialize_private_key_pem();
        
        Ok((cert_pem.into_bytes(), key_pem.into_bytes()))
    }
}