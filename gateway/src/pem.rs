//! PKCS#8 v1 + SPKI DER encoding for Ed25519 keys (RFC 8410).
//! PEM armor (RFC 7468).
//!
//! Used by both `settings.rs` (production `--dev-keys` path) and
//! `benches/auth_bench.rs` (sign+verify throughput).

use base64::Engine;

/// Construct a PKCS#8 v1 DER-encoded Ed25519 private key from a 32-byte seed.
///
/// The caller must ensure `seed` is exactly 32 bytes.
#[must_use]
pub fn ed25519_pkcs8_der(seed: &[u8]) -> Vec<u8> {
    let mut der = Vec::with_capacity(48);
    der.extend_from_slice(&[
        0x30, 0x2e, // SEQUENCE (46 bytes content)
        0x02, 0x01, 0x00, // INTEGER 0 (version v1)
        0x30, 0x05, // SEQUENCE (5 bytes)
        0x06, 0x03, 0x2b, 0x65, 0x70, // OID 1.3.101.112 (Ed25519)
        0x04, 0x22, // OCTET STRING (34 bytes)
        0x04, 0x20, // OCTET STRING (32 bytes — the seed / private key)
    ]);
    der.extend_from_slice(seed);
    der
}

/// Construct an SPKI DER-encoded Ed25519 public key from 32 raw bytes.
///
/// The caller must ensure `public_key` is exactly 32 bytes.
#[must_use]
pub fn ed25519_spki_der(public_key: &[u8]) -> Vec<u8> {
    let mut der = Vec::with_capacity(44);
    der.extend_from_slice(&[
        0x30, 0x2a, // SEQUENCE (42 bytes content)
        0x30, 0x05, // SEQUENCE (5 bytes)
        0x06, 0x03, 0x2b, 0x65, 0x70, // OID 1.3.101.112 (Ed25519)
        0x03, 0x21, 0x00, // BIT STRING (33 bytes: 0 unused bits + 32 key)
    ]);
    der.extend_from_slice(public_key);
    der
}

/// PEM-encode a DER blob (e.g., PKCS#8 or SPKI) with the given label.
///
/// Standard labels: `"PRIVATE KEY"` for PKCS#8, `"PUBLIC KEY"` for SPKI.
#[must_use]
pub fn pem_encode(label: &str, der: &[u8]) -> String {
    use std::fmt::Write;

    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    let mut pos = 0;
    let len = b64.len();
    while pos < len {
        let end = (pos + 64).min(len);
        pem.push_str(&b64[pos..end]);
        pem.push('\n');
        pos = end;
    }
    let _ = writeln!(pem, "-----END {label}-----");
    pem
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The expected 48-byte PKCS#8 v1 DER encoding for an all-zero 32-byte seed.
    /// Byte layout:
    ///   30 2e       — SEQUENCE (46 bytes content)
    ///      02 01 00 — INTEGER 0 (version v1)
    ///      30 05    — SEQUENCE (5 bytes: OID)
    ///         06 03 2b 65 70 — OID 1.3.101.112 (Ed25519)
    ///      04 22    — OCTET STRING (34 bytes)
    ///         04 20 — OCTET STRING (32 bytes — the private key)
    ///         [32 zero bytes]
    const ZERO_SEED_PKCS8: [u8; 48] = [
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00,
    ];

    #[test]
    fn pkcs8_der_zero_seed() {
        let der = ed25519_pkcs8_der(&[0u8; 32]);
        assert_eq!(der, ZERO_SEED_PKCS8);
    }
}
