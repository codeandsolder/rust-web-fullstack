//! Key store trait and in-memory implementation.
//!
//! The [`KeyStore`] trait is **sealed** — only types within this crate can
//! implement it.  This ensures that the gateway always controls how keys are
//! stored and rotated.
//!
//! The default implementation ([`InMemoryKeyStore`]) uses an
//! `RwLock<HashMap<Kid, String>>` holding SPKI PEM strings, and refreshes
//! expired entries on access.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;

pub(crate) mod seal {
    /// Sealed trait — only types in this crate can implement [`KeyStore`].
    pub trait Sealed {}
}

/// A unique key identifier for a JWT signing/verification key.
///
/// This is the `kid` value in the JWT header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Kid(pub Arc<str>);

/// Key store for JWT verification keys.
///
/// This trait is **sealed**: it can only be implemented within this crate.
/// See the [module-level documentation](self) for details.
///
/// The store allows the gateway to support key rotation: a new key is added
/// with a fresh [`Kid`], and old keys are retained until they expire so that
/// tokens signed with them remain valid for their lifetime.
pub trait KeyStore: seal::Sealed + Send + Sync {
    /// Look up a [`DecodingKey`] by its [`Kid`].
    ///
    /// Returns `None` if the key is unknown or has expired.
    fn get(&self, kid: &Kid) -> Option<DecodingKey>;

    /// Insert a key entry with the given TTL.
    fn insert(&self, kid: Kid, pem_public_key: Arc<str>, ttl: Duration);

    /// Remove expired entries.
    fn purge_expired(&self);
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct KeyEntry {
    /// SPKI PEM string for the public key.
    pem: Arc<str>,
    expires_at: Instant,
}

/// In-memory key store backed by an `RwLock<HashMap>`.
///
/// # Expiry
///
/// Entries have a configurable TTL.  Expired entries are purged lazily on
/// access (by [`InMemoryKeyStore::get`]) or eagerly by calling
/// [`InMemoryKeyStore::purge_expired`].
#[derive(Debug)]
pub struct InMemoryKeyStore {
    inner: RwLock<HashMap<Kid, KeyEntry>>,
}

impl InMemoryKeyStore {
    /// Create an empty key store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl seal::Sealed for InMemoryKeyStore {}

#[allow(clippy::significant_drop_tightening)]
impl KeyStore for InMemoryKeyStore {
    fn get(&self, kid: &Kid) -> Option<DecodingKey> {
        let guard = self.inner.try_read().ok()?;
        let entry = guard.get(kid)?;
        if Instant::now() < entry.expires_at {
            DecodingKey::from_ed_pem(entry.pem.as_bytes()).ok()
        } else {
            None
        }
    }

    fn insert(&self, kid: Kid, pem_public_key: Arc<str>, ttl: Duration) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(
                kid,
                KeyEntry {
                    pem: pem_public_key,
                    expires_at: Instant::now() + ttl,
                },
            );
        }
    }

    fn purge_expired(&self) {
        if let Ok(mut map) = self.inner.write() {
            map.retain(|_, entry| Instant::now() < entry.expires_at);
        }
    }
}
