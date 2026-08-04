//! Provider account commands (E3-07). **Secrets never cross the command
//! boundary:** `add` takes the secret once (stored in the keyring
//! server-side); every other command returns only masked/credential-ref
//! data.

use std::sync::Arc;

use ade_core::provider_accounts::{AddAccountOptions, ProviderAccount};
use serde::Serialize;
use tauri::State;

use crate::app::App;

/// Renderer-facing DTO: no secret, no credential_ref (the ref is an
/// internal keyring handle).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountDto {
    pub id: String,
    pub provider_id: String,
    pub account_id: String,
    pub label: Option<String>,
    pub is_default: bool,
    /// Server-computed mask of the keyring secret (a mask, never the
    /// secret). Falls back to a full mask when the keyring is unavailable.
    pub masked_secret: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn to_dto(account: &ProviderAccount) -> ProviderAccountDto {
    ProviderAccountDto {
        id: account.id.clone(),
        provider_id: account.provider_id.clone(),
        account_id: account.account_id.clone(),
        label: account.meta.as_ref().and_then(|m| m.label.clone()),
        is_default: account.is_default,
        masked_secret: ade_core::provider_accounts::secrets::load_secret(&account.credential_ref)
            .map(|secret| ade_core::provider_accounts::secrets::mask(&secret))
            .unwrap_or_else(|_| "••••".to_string()),
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

#[tauri::command]
pub fn add_provider_account(
    app: State<'_, Arc<App>>,
    provider_id: String,
    account_id: String,
    secret: String,
    label: Option<String>,
) -> Result<ProviderAccountDto, String> {
    let credential_ref = uuid::Uuid::new_v4().to_string();
    ade_core::provider_accounts::secrets::store_secret(&credential_ref, &secret)
        .map_err(|e| e.to_string())?;
    match app.provider_accounts.add(AddAccountOptions {
        provider_id,
        account_id,
        credential_ref: credential_ref.clone(),
        label,
    }) {
        Ok(account) => Ok(to_dto(&account)),
        Err(e) => {
            // Roll the keyring entry back when the row insert fails.
            if let Err(delete_err) =
                ade_core::provider_accounts::secrets::delete_secret(&credential_ref)
            {
                tracing::warn!(error = %delete_err, "rollback keyring delete failed");
            }
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub fn list_provider_accounts(
    app: State<'_, Arc<App>>,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccountDto>, String> {
    app.provider_accounts
        .list(provider_id.as_deref())
        .map(|accounts| accounts.iter().map(to_dto).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_provider_account(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let account = app
        .provider_accounts
        .remove(&id)
        .map_err(|e| e.to_string())?;
    if let Err(e) = ade_core::provider_accounts::secrets::delete_secret(&account.credential_ref) {
        // Row is already gone; an orphaned keyring entry is a warning, not
        // a failure (acceptance: removal must not error on secret cleanup).
        tracing::warn!(error = %e, credential_ref = %account.credential_ref, "keyring secret delete failed");
    }
    Ok(())
}

#[tauri::command]
pub fn set_default_provider_account(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    app.provider_accounts
        .set_default(&id)
        .map_err(|e| e.to_string())
}

/// Registry listing for the accounts UI (no secrets in `ProviderDto`).
#[tauri::command]
pub fn list_providers() -> Vec<ade_providers::ProviderDto> {
    ade_providers::list_dtos()
}
