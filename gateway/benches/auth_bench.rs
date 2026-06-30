//! Criterion benchmark: `EdDSA` JWT sign + verify throughput.
//!
//! Measures the end-to-end time to create a JWT with `EdDSA` (Ed25519) and
//! immediately validate it.  This exercises PEM parsing, base64 encoding,
//! base64 decoding, signature creation, and signature verification.
//!
//! # Lints
//!
//! `expect_used` and `default_trait_access` are allowed here because
//! benchmark failure is a programming error.
#![allow(clippy::expect_used, clippy::default_trait_access)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gateway_example::auth::{create_jwt, validate_jwt};
use gateway_example::pem::{ed25519_pkcs8_der, ed25519_spki_der, pem_encode};

/// Fixed seed for deterministic benchmark keypair.
const BENCH_SEED: [u8; 32] = [0x42u8; 32];

/// Deterministic Ed25519 keypair PEM strings from [`BENCH_SEED`].
fn dev_keypair_pems() -> (String, String) {
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    let key_pair =
        Ed25519KeyPair::from_seed_unchecked(&BENCH_SEED).expect("Ed25519 keypair from seed");
    let public_key = key_pair.public_key().as_ref().to_vec();
    let private_pem = pem_encode("PRIVATE KEY", &ed25519_pkcs8_der(&BENCH_SEED));
    let public_pem = pem_encode("PUBLIC KEY", &ed25519_spki_der(&public_key));
    (private_pem, public_pem)
}

fn bench_sign_verify(c: &mut Criterion) {
    let (private_pem, public_pem) = dev_keypair_pems();

    c.bench_function("sign_verify", |b| {
        b.iter(|| {
            let token = create_jwt("bench-user", black_box(&private_pem))
                .expect("create_jwt should succeed");
            let claims = validate_jwt(black_box(&token), black_box(&public_pem))
                .expect("validate_jwt should succeed");
            black_box(claims.sub);
        });
    });
}

criterion_group!(benches, bench_sign_verify);
criterion_main!(benches);
