//! Device key pairs (ECDSA P-256) compatible with Node's PEM/DER defaults:
//! SPKI public PEM, PKCS#8 private PEM, DER-encoded SHA-256 signatures.
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use p256::{
    ecdsa::{signature::Signer, Signature, SigningKey},
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding},
};

pub fn generate() -> Result<(String, String), String> {
    let key = SigningKey::random(&mut rand_core::OsRng);
    let private = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|_| "无法生成设备私钥")?
        .to_string();
    let public = key
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_| "无法生成设备公钥")?;
    Ok((public, private))
}

/// base64(DER(ECDSA-SHA256(message))), as `crypto.sign('sha256', …)` produces.
pub fn sign(private_pem: &str, message: &[u8]) -> Result<String, String> {
    let key = SigningKey::from_pkcs8_pem(private_pem).map_err(|_| "设备私钥格式无效")?;
    let signature: Signature = key.sign(message);
    Ok(B64.encode(signature.to_der().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::{
        ecdsa::{signature::Verifier, VerifyingKey},
        pkcs8::DecodePublicKey,
    };
    #[test]
    fn pem_keys_sign_verifiable_der() {
        let (public, private) = generate().unwrap();
        assert!(public.starts_with("-----BEGIN PUBLIC KEY-----"));
        assert!(private.starts_with("-----BEGIN PRIVATE KEY-----"));
        let signature = sign(&private, b"canonical").unwrap();
        let der = B64.decode(signature).unwrap();
        let verifying = VerifyingKey::from_public_key_pem(&public).unwrap();
        let parsed = Signature::from_der(&der).unwrap();
        assert!(verifying.verify(b"canonical", &parsed).is_ok());
        assert!(sign("not a key", b"x").is_err());
    }
}
