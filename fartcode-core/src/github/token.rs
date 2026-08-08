//! GitHub token source (E4-07, #47): keyring entry + `gh auth token`
//! import. Zero tokens in SQLite or logs — the keyring is the only store,
//! and nothing here ever prints the secret (see `mask` for display).
//!
//! Resolution order for API calls ([`resolve_token`]): keyring entry →
//! live `gh auth token` (the gh CLI is already authenticated on dev
//! machines; this keeps the PR tab working without a manual import).

use crate::Error;

const SERVICE: &str = "fartCode";
const ACCOUNT: &str = "github-token";

/// Reads the stored token. Missing entry → `Ok(None)` (never an error —
/// "no token" is a normal state the UI renders as an empty-state CTA).
pub fn get_token() -> Result<Option<String>, Error> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::CredentialStore(format!("keyring read: {e}"))),
    }
}

/// Stores a token (overwrite). Empty/whitespace tokens are rejected.
pub fn set_token(token: &str) -> Result<(), Error> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::GithubAuth("token is empty".into()));
    }
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    entry
        .set_password(token)
        .map_err(|e| Error::CredentialStore(format!("keyring store: {e}")))
}

/// Removes the stored token (missing entry is fine).
pub fn clear_token() -> Result<(), Error> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| Error::CredentialStore(format!("keyring entry: {e}")))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::CredentialStore(format!("keyring delete: {e}"))),
    }
}

/// Copies the gh CLI's active token into our keyring (ticket: "gh auth
/// token import helper"). Returns the imported token's mask — never the
/// token itself.
pub fn import_from_gh() -> Result<String, Error> {
    let gh = crate::pty::launcher::find_on_path("gh").ok_or_else(|| {
        Error::GithubAuth("gh CLI not found on PATH — install it or paste a token manually".into())
    })?;
    let output = std::process::Command::new(&gh)
        .args(["auth", "token"])
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .map_err(|e| Error::Github(format!("failed to run gh auth token: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::GithubAuth(format!(
            "gh auth token failed — is gh authenticated? ({})",
            stderr.trim()
        )));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(Error::GithubAuth("gh returned an empty token".into()));
    }
    set_token(&token)?;
    Ok(mask(&token))
}

/// Keyring → gh fallback. `Ok(None)` = no token anywhere (UI shows the
/// "connect GitHub" empty state; never an error for a missing token).
pub fn resolve_token() -> Result<Option<String>, Error> {
    if let Some(token) = get_token()? {
        return Ok(Some(token));
    }
    // Best-effort gh fallback: a failed `gh auth token` (not logged in,
    // no gh) is just "no token", not a fatal error.
    let gh = match crate::pty::launcher::find_on_path("gh") {
        Some(gh) => gh,
        None => return Ok(None),
    };
    let output = std::process::Command::new(&gh)
        .args(["auth", "token"])
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .map_err(|e| Error::Github(format!("failed to run gh auth token: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

/// Display mask (`ghpa…`) — the only token-derived string ever shown.
pub fn mask(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 4 {
        "••••".to_string()
    } else {
        format!("{}…", chars[..4].iter().collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_keeps_prefix() {
        assert_eq!(mask("ghp_abcdef123456"), "ghp_…");
        assert_eq!(mask("abc"), "••••");
        assert_eq!(mask(""), "••••");
    }

    #[test]
    fn set_rejects_empty_token() {
        assert!(matches!(set_token("   "), Err(Error::GithubAuth(_))));
    }

    #[test]
    fn keyring_round_trip_or_typed_error() {
        // Same contract as provider_accounts::secrets: on machines without a
        // keyring service this surfaces the typed error instead of panicking.
        match set_token("ghp_test_round_trip") {
            Ok(()) => {
                assert_eq!(get_token().unwrap().as_deref(), Some("ghp_test_round_trip"));
                clear_token().unwrap();
                assert!(get_token().unwrap().is_none());
            }
            Err(Error::CredentialStore(_)) => {}
            Err(other) => panic!("expected CredentialStore, got {other:?}"),
        }
    }
}
