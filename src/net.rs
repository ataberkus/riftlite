//! HTTP agents.
//!
//! The LCU (`127.0.0.1:<random port>`) and Live Client Data API (`127.0.0.1:2999`)
//! serve certificates issued by Riot's "LoL Game Engineering Certificate
//! Authority" (published at static.developer.riotgames.com/docs/lol/riotgames.pem).
//! Blitz disabled TLS verification for these (`rejectUnauthorized: false`);
//! riftlite instead pins that CA: the leaf must be issued by it, carry a valid
//! signature from its key and be within its validity window. Hostname matching
//! is skipped because the leaf names Riot's own loopback identity and the
//! socket is always to 127.0.0.1.

use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, SignatureScheme};
use x509_parser::prelude::{FromDer, X509Certificate};

const RIOT_CA_PEM: &[u8] = include_bytes!("../assets/riotgames.pem");

#[derive(Debug)]
struct RiotCaVerifier {
    ca_der: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for RiotCaVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let bad = |e| Error::InvalidCertificate(e);
        let (_, ca) = X509Certificate::from_der(&self.ca_der).map_err(|_| bad(CertificateError::BadEncoding))?;
        let (_, leaf) = X509Certificate::from_der(end_entity.as_ref()).map_err(|_| bad(CertificateError::BadEncoding))?;
        if leaf.issuer().as_raw() != ca.subject().as_raw() {
            return Err(bad(CertificateError::UnknownIssuer));
        }
        leaf.verify_signature(Some(ca.public_key())).map_err(|_| bad(CertificateError::BadSignature))?;
        let t = now.as_secs() as i64;
        let v = leaf.validity();
        if t < v.not_before.timestamp() {
            return Err(bad(CertificateError::NotValidYet));
        }
        if t > v.not_after.timestamp() {
            return Err(bad(CertificateError::Expired));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn riot_ca_der() -> Vec<u8> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(RIOT_CA_PEM).expect("embedded Riot CA PEM is valid");
    pem.contents
}

/// Agent for Riot's loopback APIs, pinned to the Riot CA.
pub fn riot_local_agent() -> ureq::Agent {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = RiotCaVerifier { ca_der: riot_ca_der(), provider: provider.clone() };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        .tls_config(Arc::new(config))
        .timeout_connect(Duration::from_millis(800))
        .timeout(Duration::from_secs(5))
        .build()
}

/// Agent for public HTTPS (Data Dragon), standard WebPKI verification.
pub fn public_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("riftlite/", env!("CARGO_PKG_VERSION")))
        .build()
}

pub fn basic_auth(user: &str, password: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = format!("{user}:{password}");
    let mut out = String::with_capacity(6 + input.len().div_ceil(3) * 4);
    out.push_str("Basic ");
    for chunk in input.as_bytes().chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic_auth_matches_rfc4648_padding() {
        assert_eq!(super::basic_auth("riot", "a"), "Basic cmlvdDph");
        assert_eq!(super::basic_auth("riot", "ab"), "Basic cmlvdDphYg==");
        assert_eq!(super::basic_auth("riot", "abc"), "Basic cmlvdDphYmM=");
    }
}
