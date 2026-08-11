//! SSH profile secrets (E12-03): passwords and key passphrases live in the OS
//! keyring, never in the DB, DTOs, or logs.
//!
//! Service name: `fartCode` · entry name: `ssh-connection:<connection id>`.
//!
//! Same contract as [`crate::provider_accounts::secrets`]: a missing or broken
//! keyring is a typed error, never a silent plaintext fallback.

use crate::Error;

const SERVICE: &str = "fartCode";

/// Keyring entry name for a connection id. Safe to log — it is a lookup key.
pub fn entry_name(connection_id: &str) -> String {
    format!("ssh-connection:{connection_id}")
}

fn entry(connection_id: &str) -> Result<keyring::Entry, Error> {
    keyring::Entry::new(SERVICE, &entry_name(connection_id))
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))
}

/// Stores (or overwrites) the secret for a connection.
pub fn store_secret(connection_id: &str, secret: &str) -> Result<(), Error> {
    entry(connection_id)?
        .set_password(secret)
        .map_err(|e| Error::CredentialStore(format!("keyring store: {e}")))
}

/// Reads the secret. Missing entry → [`Error::CredentialSecretMissing`].
pub fn load_secret(connection_id: &str) -> Result<String, Error> {
    entry(connection_id)?.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => Error::CredentialSecretMissing(entry_name(connection_id)),
        other => Error::CredentialStore(format!("keyring read: {other}")),
    })
}

/// Whether a secret exists for this connection.
///
/// The UI needs to tell “no password saved” from “one is stored, leave the
/// field blank to keep it”. Returns a boolean and never the secret; a keyring
/// that cannot be read at all reads as “none”, the same way a missing entry
/// does — both mean “the user will have to type it”.
pub fn has_secret(connection_id: &str) -> bool {
    load_secret(connection_id).is_ok()
}

/// Deletes the secret. Missing entries are OK (delete is idempotent).
pub fn delete_secret(connection_id: &str) -> Result<(), Error> {
    match entry(connection_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::CredentialStore(format!("keyring delete: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_name_is_namespaced() {
        assert_eq!(entry_name("abc"), "ssh-connection:abc");
        assert_ne!(entry_name("abc"), "provider-account:abc");
    }
}
