//! Credential storage (E3-07): secrets live in the OS keyring (keyring
//! crate), never in the DB or logs. The `provider_accounts` table holds a
//! `credential_ref` string; this module maps refs to keyring entries.
//!
//! Service name: `fartCode` · entry name: `provider-account:<credential_ref>`.
//!
//! **Unavailability contract:** a missing/broken keyring is a typed error
//! (`Error::CredentialStore`), never silent plaintext fallback (PRD §7:
//! secrets — keyring only).

use crate::Error;

const SERVICE: &str = "fartCode";

fn entry_name(credential_ref: &str) -> String {
    format!("provider-account:{credential_ref}")
}

/// Stores a secret under `credential_ref`. Overwrites any existing entry.
pub fn store_secret(credential_ref: &str, secret: &str) -> Result<(), Error> {
    let entry = keyring::Entry::new(SERVICE, &entry_name(credential_ref))
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    entry
        .set_password(secret)
        .map_err(|e| Error::CredentialStore(format!("keyring store: {e}")))
}

/// Reads the secret for `credential_ref`. Missing entry →
/// [`Error::CredentialSecretMissing`].
pub fn load_secret(credential_ref: &str) -> Result<String, Error> {
    let entry = keyring::Entry::new(SERVICE, &entry_name(credential_ref))
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => Error::CredentialSecretMissing(credential_ref.to_string()),
        other => Error::CredentialStore(format!("keyring read: {other}")),
    })
}

/// Deletes the secret for `credential_ref`. Missing entries are OK.
pub fn delete_secret(credential_ref: &str) -> Result<(), Error> {
    let entry = keyring::Entry::new(SERVICE, &entry_name(credential_ref))
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    entry.delete_credential().map_err(|e| match e {
        keyring::Error::NoEntry => Error::CredentialSecretMissing(credential_ref.to_string()),
        other => Error::CredentialStore(format!("keyring delete: {other}")),
    })
}

/// Masks a secret for display: first 4 chars + `…`. Values of 4 chars or
/// fewer mask fully.
pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 4 {
        "••••".to_string()
    } else {
        format!("{}…", chars[..4].iter().collect::<String>())
    }
}

/// Masks every value in an env list — used as the log-safe rendering of
/// `resolve_env` output (acceptance: no secret in logs).
pub fn redact_env(env: &[(String, String)]) -> Vec<(String, String)> {
    env.iter().map(|(k, v)| (k.clone(), mask(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_keeps_prefix() {
        assert_eq!(mask("sk-ant-1234567890"), "sk-a…");
        assert_eq!(mask("abcd"), "••••");
        assert_eq!(mask(""), "••••");
    }

    #[test]
    fn redact_env_masks_values_only() {
        let redacted = redact_env(&[
            ("ANTHROPIC_API_KEY".into(), "sk-ant-secretvalue".into()),
            ("PATH".into(), "/usr/bin".into()),
        ]);
        assert_eq!(redacted[0], ("ANTHROPIC_API_KEY".into(), "sk-a…".into()));
        assert_eq!(redacted[1], ("PATH".into(), "/usr…".into()));
    }

    #[test]
    fn keyring_round_trip_or_typed_error() {
        // On CI/containers without a keyring service this must surface the
        // typed CredentialStore error — never panic, never fall back.
        let result = store_secret("test-round-trip", "hunter2");
        match result {
            Ok(()) => {
                assert_eq!(load_secret("test-round-trip").unwrap(), "hunter2");
                delete_secret("test-round-trip").unwrap();
            }
            Err(Error::CredentialStore(_)) => {}
            Err(other) => panic!("expected CredentialStore, got {other:?}"),
        }
    }
}
