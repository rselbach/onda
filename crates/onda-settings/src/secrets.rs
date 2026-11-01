//! The OpenRouter API key in the OS credential store (CLN-23, CLN-24).
//!
//! The key is carried as [`ApiKey`], whose `Debug` is redacted, and is never
//! logged (CLN-8, LOG-2).

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use onda_domain::cleanup::ApiKey;

/// Credential-store service, the bundle identifier (ARCH-13).
pub const SERVICE: &str = "com.rselbach.onda";

/// Credential-store account holding the OpenRouter key.
pub const OPENROUTER_ACCOUNT: &str = "openRouterApiKey";

/// The credential-store operation that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretOp {
    /// Reading the stored key.
    Read,
    /// Creating or replacing the stored key.
    Save,
    /// Deleting the stored key.
    Delete,
}

impl SecretOp {
    fn verb(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Save => "save",
            Self::Delete => "delete",
        }
    }
}

/// A credential-store failure. `Display` is the inline copy shown under the
/// key field (CLN-24), e.g. "Keychain read failed (OSStatus -25293)."; the
/// status is omitted when the store reported none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretError {
    op: SecretOp,
    status: Option<i32>,
}

impl SecretError {
    /// An error for `op`, with the OS status code when known.
    pub fn new(op: SecretOp, status: Option<i32>) -> SecretError {
        SecretError { op, status }
    }

    /// The operation that failed.
    pub fn op(&self) -> SecretOp {
        self.op
    }

    /// The OS status code (an `OSStatus` on macOS), when the store gave one.
    pub fn status(&self) -> Option<i32> {
        self.status
    }
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verb = self.op.verb();
        match self.status {
            Some(status) => write!(f, "Keychain {verb} failed (OSStatus {status})."),
            None => write!(f, "Keychain {verb} failed."),
        }
    }
}

impl std::error::Error for SecretError {}

/// Where the OpenRouter key lives (CLN-23, CLN-24).
///
/// A missing key is `Ok(None)`, not an error, and deleting a missing key
/// succeeds. Implementations must never log or `Debug`-print the key.
pub trait SecretStore: Send + Sync {
    /// The stored key, or `None` when none (or a blank one) is stored.
    fn read(&self) -> Result<Option<ApiKey>, SecretError>;

    /// Creates or replaces the stored key.
    fn write(&self, key: &ApiKey) -> Result<(), SecretError>;

    /// Deletes the stored key; succeeds when none is stored.
    fn delete(&self) -> Result<(), SecretError>;

    /// Saves what the user typed (CLN-24): trims it, and a blank value
    /// deletes the stored key.
    fn save(&self, value: &str) -> Result<(), SecretError> {
        match ApiKey::new(value) {
            Ok(key) => self.write(&key),
            Err(_) => self.delete(),
        }
    }
}

/// The OS credential store through the `keyring` crate: Keychain on macOS
/// (service [`SERVICE`], account [`OPENROUTER_ACCOUNT`]).
///
/// On macOS `keyring` writes a generic password to the login keychain and
/// offers no accessibility attribute. The login keychain unlocks at login,
/// which gives the "readable after first unlock" behavior CLN-23 asks for.
#[derive(Debug, Clone)]
pub struct KeychainSecretStore {
    service: String,
}

impl Default for KeychainSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeychainSecretStore {
    /// The store for Onda's OpenRouter key.
    pub fn new() -> KeychainSecretStore {
        KeychainSecretStore {
            service: SERVICE.to_owned(),
        }
    }

    #[cfg(test)]
    fn with_service(service: &str) -> KeychainSecretStore {
        KeychainSecretStore {
            service: service.to_owned(),
        }
    }

    fn entry(&self, op: SecretOp) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, OPENROUTER_ACCOUNT).map_err(|error| failure(op, &error))
    }
}

impl SecretStore for KeychainSecretStore {
    fn read(&self) -> Result<Option<ApiKey>, SecretError> {
        match self.entry(SecretOp::Read)?.get_password() {
            Ok(value) => Ok(ApiKey::new(&value).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(failure(SecretOp::Read, &error)),
        }
    }

    fn write(&self, key: &ApiKey) -> Result<(), SecretError> {
        self.entry(SecretOp::Save)?
            .set_password(key.expose())
            .map_err(|error| failure(SecretOp::Save, &error))
    }

    fn delete(&self) -> Result<(), SecretError> {
        match self.entry(SecretOp::Delete)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(failure(SecretOp::Delete, &error)),
        }
    }
}

// keyring's Display never includes the secret (its Debug can, for
// undecodable bytes), so only Display is logged
fn failure(op: SecretOp, error: &keyring::Error) -> SecretError {
    let status = os_status(error);
    tracing::error!(op = op.verb(), status, %error, "keychain operation failed");
    SecretError::new(op, status)
}

#[cfg(target_os = "macos")]
fn os_status(error: &keyring::Error) -> Option<i32> {
    let platform = match error {
        keyring::Error::PlatformFailure(platform) | keyring::Error::NoStorageAccess(platform) => {
            platform
        }
        _ => return None,
    };
    platform
        .downcast_ref::<security_framework::base::Error>()
        .map(|error| error.code())
}

#[cfg(not(target_os = "macos"))]
fn os_status(_error: &keyring::Error) -> Option<i32> {
    None
}

/// In-memory [`SecretStore`] for tests. Optionally fails one operation with
/// a given error, to exercise the inline error paths (CLN-24, CLN-25).
#[derive(Default)]
pub struct MemorySecretStore {
    key: Mutex<Option<String>>,
    failure: Mutex<Option<SecretError>>,
}

impl fmt::Debug for MemorySecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemorySecretStore")
            .field("stored", &lock(&self.key).is_some())
            .field("failure", &*lock(&self.failure))
            .finish()
    }
}

impl MemorySecretStore {
    /// An empty store.
    pub fn new() -> MemorySecretStore {
        MemorySecretStore::default()
    }

    /// Makes every later operation matching `error.op()` fail with `error`;
    /// `None` stops failing.
    pub fn fail_with(&self, error: Option<SecretError>) {
        *lock(&self.failure) = error;
    }

    fn check(&self, op: SecretOp) -> Result<(), SecretError> {
        match *lock(&self.failure) {
            Some(error) if error.op() == op => Err(error),
            _ => Ok(()),
        }
    }
}

impl SecretStore for MemorySecretStore {
    fn read(&self) -> Result<Option<ApiKey>, SecretError> {
        self.check(SecretOp::Read)?;
        Ok(lock(&self.key)
            .as_deref()
            .and_then(|value| ApiKey::new(value).ok()))
    }

    fn write(&self, key: &ApiKey) -> Result<(), SecretError> {
        self.check(SecretOp::Save)?;
        *lock(&self.key) = Some(key.expose().to_owned());
        Ok(())
    }

    fn delete(&self) -> Result<(), SecretError> {
        self.check(SecretOp::Delete)?;
        *lock(&self.key) = None;
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn exposed(store: &dyn SecretStore) -> Option<String> {
        store
            .read()
            .expect("read")
            .map(|key| key.expose().to_owned())
    }

    #[test]
    fn memory_store_semantics_cln_24() {
        let store = MemorySecretStore::new();
        assert_eq!(exposed(&store), None, "starts empty");
        store
            .delete()
            .expect("deleting a missing key is not an error");

        store.save("  sk-or-troy-barnes \n").expect("save");
        assert_eq!(
            exposed(&store).as_deref(),
            Some("sk-or-troy-barnes"),
            "save trims"
        );

        store.save("sk-or-abed-nadir").expect("overwrite");
        assert_eq!(
            exposed(&store).as_deref(),
            Some("sk-or-abed-nadir"),
            "save replaces"
        );

        store.save("   ").expect("blank save");
        assert_eq!(exposed(&store), None, "blank save deletes");
        store.save("").expect("empty save on a missing key");
        assert_eq!(exposed(&store), None);
    }

    #[test]
    fn memory_store_never_debug_prints_the_key_cln_8() {
        let store = MemorySecretStore::new();
        store.save("sk-or-greendale-secret").expect("save");
        let key = store.read().expect("read").expect("stored");
        for (name, got) in [("store", format!("{store:?}")), ("key", format!("{key:?}"))] {
            assert!(
                !got.contains("greendale-secret"),
                "{name} Debug leaked the key: {got}"
            );
        }
    }

    #[test]
    fn memory_store_fails_only_the_chosen_op() {
        let store = MemorySecretStore::new();
        store.save("sk-or-dean-pelton").expect("save");
        let error = SecretError::new(SecretOp::Read, Some(-25293));
        store.fail_with(Some(error));
        assert_eq!(store.read().map(|_| ()), Err(error));
        store.save("sk-or-dean-pelton-2").expect("save still works");
        store.fail_with(Some(SecretError::new(SecretOp::Delete, None)));
        assert!(store.save("  ").is_err(), "blank save goes through delete");
        store.fail_with(None);
        assert_eq!(exposed(&store).as_deref(), Some("sk-or-dean-pelton-2"));
    }

    #[test]
    fn error_copy_cln_24() {
        let cases = [
            (
                "read with status",
                SecretError::new(SecretOp::Read, Some(-25293)),
                "Keychain read failed (OSStatus -25293).",
            ),
            (
                "save with status",
                SecretError::new(SecretOp::Save, Some(-25299)),
                "Keychain save failed (OSStatus -25299).",
            ),
            (
                "delete with status",
                SecretError::new(SecretOp::Delete, Some(-61)),
                "Keychain delete failed (OSStatus -61).",
            ),
            (
                "read without status",
                SecretError::new(SecretOp::Read, None),
                "Keychain read failed.",
            ),
        ];
        for (name, error, want) in cases {
            assert_eq!(error.to_string(), want, "{name}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn os_status_comes_from_the_security_framework_error() {
        let cases = [
            (
                "platform failure",
                keyring::Error::PlatformFailure(Box::new(
                    security_framework::base::Error::from_code(-25293),
                )),
                Some(-25293),
            ),
            (
                "no storage access",
                keyring::Error::NoStorageAccess(Box::new(
                    security_framework::base::Error::from_code(-61),
                )),
                Some(-61),
            ),
            (
                "bad encoding",
                keyring::Error::BadEncoding(Vec::new()),
                None,
            ),
            ("no default store", keyring::Error::NoDefaultStore, None),
        ];
        for (name, error, want) in cases {
            assert_eq!(os_status(&error), want, "{name}");
        }
    }

    // run with `cargo test -p onda-settings -- --ignored`; uses a throwaway
    // service, never the real one, and deletes its item even on failure
    #[test]
    #[ignore = "touches the real login keychain"]
    fn real_keychain_round_trip() {
        struct Cleanup(KeychainSecretStore);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Err(error) = self.0.delete() {
                    eprintln!("cleanup failed: {error}");
                }
            }
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let service = format!("com.rselbach.onda.tests.{}.{nanos}", std::process::id());
        assert_ne!(service, SERVICE);
        let store = Cleanup(KeychainSecretStore::with_service(&service));
        let store = &store.0;

        assert_eq!(exposed(store), None, "absent at start");
        store
            .delete()
            .expect("deleting a missing key is not an error");

        store.save("  sk-or-troy-barnes  ").expect("save");
        assert_eq!(
            exposed(store).as_deref(),
            Some("sk-or-troy-barnes"),
            "save trims"
        );

        store.save("sk-or-abed-nadir").expect("overwrite");
        assert_eq!(
            exposed(store).as_deref(),
            Some("sk-or-abed-nadir"),
            "save replaces"
        );

        store.save(" ").expect("blank save");
        assert_eq!(exposed(store), None, "blank save deletes");
        store.delete().expect("second delete");
    }
}
