//! Criterion benchmark: `EdDSA` JWT sign + verify throughput.
//!
//! Measures the end-to-end time to create a JWT with `EdDSA` (Ed25519) and
//! immediately validate it.  This exercises PEM parsing, base64 encoding,
//! base64 decoding, signature creation, and signature verification.
//!
use std::fmt::Debug;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gateway_example::auth::{create_jwt, validate_jwt};
use gateway_example::pem::{ed25519_pkcs8_der, ed25519_spki_der, pem_encode};
use jsonwebtoken::{DecodingKey, EncodingKey};
use rwf_domain::UserId;
use uuid::Uuid;

/// Fixed seed for deterministic benchmark keypair.
const BENCH_SEED: [u8; 32] = [0x42u8; 32];

/// Deterministic Ed25519 keypair PEM strings from [`BENCH_SEED`].
fn or_exit<T, E: Debug>(operation: &str, result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("FATAL: {operation}: {error:?}");
            std::process::exit(1);
        }
    }
}

fn dev_keypair_pems() -> (String, String) {
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    let key_pair = or_exit(
        "construct deterministic Ed25519 keypair",
        Ed25519KeyPair::from_seed_unchecked(&BENCH_SEED),
    );
    let public_key = key_pair.public_key().as_ref().to_vec();
    let private_pem = pem_encode("PRIVATE KEY", &ed25519_pkcs8_der(&BENCH_SEED));
    let public_pem = pem_encode("PUBLIC KEY", &ed25519_spki_der(&public_key));
    (private_pem, public_pem)
}

fn bench_sign_verify(c: &mut Criterion) {
    let (private_pem, public_pem) = dev_keypair_pems();
    let encoding_key = or_exit(
        "parse benchmark Ed25519 private key",
        EncodingKey::from_ed_pem(private_pem.as_bytes()),
    );
    let decoding_key = or_exit(
        "parse benchmark Ed25519 public key",
        DecodingKey::from_ed_pem(public_pem.as_bytes()),
    );

    let bench_user_id = or_exit(
        "construct non-zero benchmark user id",
        UserId::try_from(Uuid::from_u128(0xDEAD_BEEF_CAFE_BABE_0123_4567_89AB_CDEF)),
    );

    c.bench_function("sign_verify", |b| {
        b.iter(|| {
            let token = or_exit(
                "create benchmark JWT",
                create_jwt(
                    black_box(&bench_user_id),
                    black_box(&encoding_key),
                    black_box(60 * 60 * 24),
                ),
            );
            let claims = or_exit(
                "validate benchmark JWT",
                validate_jwt(black_box(&token), black_box(&decoding_key)),
            );
            let _ = black_box(claims.sub);
        });
    });
}

criterion_group!(benches, bench_sign_verify);
criterion_main!(benches);
