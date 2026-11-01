//! Typed Onda settings and the OpenRouter API key.
//!
//! [`Settings`] owns `<data>/settings.json` (SET-1): one JSON object, one key
//! per setting ([`Key`]), written atomically. Reads are total (SET-2) and
//! computed defaults are never written back (SET-2a).
//!
//! [`SecretStore`] holds the OpenRouter key, which is not a setting (SET-6,
//! CLN-23). [`KeychainSecretStore`] backs it with the OS credential store;
//! [`MemorySecretStore`] is the in-memory fake for tests.

mod key;
mod secrets;
mod store;

pub use key::Key;
pub use secrets::{
    KeychainSecretStore, MemorySecretStore, OPENROUTER_ACCOUNT, SERVICE, SecretError, SecretOp,
    SecretStore,
};
pub use store::{Settings, SettingsError};
