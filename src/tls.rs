use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Once;

use anyhow::{anyhow, Context, Result};
use rustls::pki_types::CertificateDer;
use rustls::ServerConfig;

/// Build a TLS [`ServerConfig`] from a PEM cert + key on disk.
///
/// The returned config has client auth disabled and uses the
/// aws-lc-rs rustls crypto provider.
pub fn load_tls_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig> {
    install_default_crypto_provider();

    let cert_file = File::open(cert_path)
        .with_context(|| format!("opening TLS cert at {}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing TLS cert at {}", cert_path.display()))?;
    if certs.is_empty() {
        return Err(anyhow!("no certificates found in {}", cert_path.display()));
    }

    let key_file = File::open(key_path)
        .with_context(|| format!("opening TLS key at {}", key_path.display()))?;
    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .with_context(|| format!("parsing TLS key at {}", key_path.display()))?
        .ok_or_else(|| anyhow!("no private key found in {}", key_path.display()))?;

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("constructing rustls ServerConfig")
}

fn install_default_crypto_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // rustls 0.23 requires the application to pick a CryptoProvider.
        // aws-lc-rs is enabled by rustls's default features.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_self_signed_pair() -> (tempfile::NamedTempFile, tempfile::NamedTempFile) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("rcgen self-signed cert");

        let mut cert_file = tempfile::NamedTempFile::new().expect("cert tempfile");
        cert_file
            .write_all(cert.cert.pem().as_bytes())
            .expect("write cert pem");

        let mut key_file = tempfile::NamedTempFile::new().expect("key tempfile");
        key_file
            .write_all(cert.key_pair.serialize_pem().as_bytes())
            .expect("write key pem");

        (cert_file, key_file)
    }

    #[test]
    fn loads_self_signed_pem_pair() {
        let (cert_file, key_file) = write_self_signed_pair();
        let result = load_tls_config(cert_file.path(), key_file.path());
        assert!(
            result.is_ok(),
            "expected load_tls_config to succeed, got {:?}",
            result.err()
        );
    }

    #[test]
    fn errors_when_cert_path_missing() {
        let (_cert_file, key_file) = write_self_signed_pair();
        let missing = std::path::Path::new("/nonexistent/path/to/cert.pem");
        let result = load_tls_config(missing, key_file.path());
        assert!(result.is_err());
    }
}
