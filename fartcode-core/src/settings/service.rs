//! Settings service (ARCHITECTURE.md §2, §6.2).
//!
//! Two layers, ported from the reference (`settings-service.ts`,
//! `db-project-settings-provider.ts`, `effective-task-settings.ts`):
//!
//! 1. **App settings** (`app_settings` table): typed keys with defaults,
//!    delta-vs-defaults writes (updating to the default deletes the row),
//!    deep-merged reads.
//! 2. **Project settings** (`project_settings` table + repo `.fartCode.json`):
//!    base (DB-backed) fields and a shareable subset; effective precedence
//!    is `defaults < .fartCode.json < DB-shareable` (later source wins, matching
//!    the reference's `mergeShareableProjectSettings(defaults, file, local)`
//!    — a local UI value overrides the file, clearing it falls back to the
//!    file, clearing the file falls back to defaults). "Share with team"
//!    moves local values into `.fartCode.json` and clears them from the DB.
//!
//! Trait note: ARCHITECTURE §6.2 sketches generic `get<T>(SettingKey<T>)`
//! methods, but §7 stores the store as `Arc<dyn SettingsStore>` — generic
//! methods are not object-safe. The trait therefore exposes JSON; typed
//! `get`/`set` wrappers live on the concrete `DbSettingsStore`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::db::Db;
use crate::settings::registry::{
    self, BaseProjectSettings, DefaultBranch, ProjectSettings, SettingKey, SettingValue,
    ShareableProjectSettings, DEFAULT_BASE_REMOTE, DEFAULT_BRANCH_FALLBACK,
    DEFAULT_PRESERVE_PATTERNS, LEGACY_CONFIG_FILE, PROJECT, PROJECT_CONFIG_FILE,
};
use crate::Error;

/// Object-safe settings store surface (§6.2, §7).
pub trait SettingsStore: Send + Sync {
    /// Effective value for an app-setting key (deep-merged with defaults).
    fn get_json(&self, key: &str) -> Result<serde_json::Value, Error>;

    /// Validates and stores `value`, computing the delta vs defaults and
    /// deleting the row when the delta is empty.
    fn set_json(&self, key: &str, value: serde_json::Value) -> Result<(), Error>;

    /// Clear all local overrides. `None` clears every `app_settings` row;
    /// `Some(project_id)` clears the project's `project_settings` row (the
    /// next read re-seeds defaults; `.fartCode.json` is untouched).
    fn reset(&self, project_id: Option<&str>) -> Result<(), Error>;

    /// Moves the project's local shareable values into its repo `.fartCode.json`
    /// and clears them from the DB.
    fn share_with_team(&self, project_id: &str) -> Result<bool, Error>;

    // -- Project settings (used by `projects` — E1-03) ----------------------

    /// Seeds a `project_settings` row with defaults (idempotent).
    fn seed_project_settings(&self, project_id: &str, repo_dir: &Path) -> Result<(), Error>;

    /// Effective project settings (defaults < `.fartCode.json` < DB-shareable).
    fn get_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<ProjectSettings, Error>;

    /// Stores local project settings (full-replace — see the concrete type).
    fn update_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
        settings: &ProjectSettings,
    ) -> Result<(), Error>;

    /// Runs the legacy `.emdash.json` migration once (idempotent).
    fn migrate_legacy_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<(), Error>;
}

/// Concrete settings store backed by SQLite (`app_settings` +
/// `project_settings` tables).
#[derive(Clone)]
pub struct DbSettingsStore {
    db: Arc<dyn Db>,
}

impl DbSettingsStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, Error>) -> Result<T, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        f(&conn)
    }

    // -- Typed app-settings access -----------------------------------------

    /// Typed read: defaults deep-merged with the stored delta.
    pub fn get<T: SettingValue>(&self, key: &SettingKey<T>) -> Result<T, Error> {
        let json = self.get_json(key.name())?;
        serde_json::from_value(json).map_err(|e| Error::InvalidSettingValue {
            key: key.name().into(),
            reason: e.to_string(),
        })
    }

    /// Typed write: stores the delta vs defaults (row deleted when empty).
    pub fn set<T: SettingValue>(&self, key: &SettingKey<T>, value: T) -> Result<(), Error> {
        let json = serde_json::to_value(value)?;
        self.set_json(key.name(), json)
    }

    // -- Project settings ---------------------------------------------------

    /// Seeds a `project_settings` row (idempotent) with base defaults and the
    /// default preserve patterns — unless the repo `.fartCode.json` already
    /// defines `preservePatterns`, in which case the shareable blob starts
    /// empty so the file wins. Called by project creation (E1-03).
    ///
    /// Note: when no `.fartCode.json` exists at seed time, the default patterns
    /// are materialized into the DB shareable blob, which then beats a
    /// later-created `.fartCode.json` until the local value is cleared — parity
    /// with the reference's `ensureRow`.
    pub fn seed_project_settings(&self, project_id: &str, repo_dir: &Path) -> Result<(), Error> {
        let file_shareable = self.read_ade_json(repo_dir);
        let shareable = if file_shareable
            .as_ref()
            .and_then(|s| s.preserve_patterns.as_ref())
            .is_some()
        {
            ShareableProjectSettings::default()
        } else {
            ShareableProjectSettings {
                preserve_patterns: Some(
                    DEFAULT_PRESERVE_PATTERNS
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
                ..Default::default()
            }
        };
        let base = BaseProjectSettings {
            default_branch: Some(DefaultBranch::Name(DEFAULT_BRANCH_FALLBACK.into())),
            base_remote: Some(DEFAULT_BASE_REMOTE.into()),
            tmux: Some(self.get(&PROJECT).unwrap_or_default().tmux_by_default),
            ..Default::default()
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO project_settings
                     (project_id, base_project_settings_json, shareable_project_settings_json)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![project_id, compact_json(&base)?, compact_json(&shareable)?],
            )?;
            Ok(())
        })
    }

    /// Effective project settings: base (DB) merged with shareable computed
    /// as `merge(defaults, .fartCode.json, DB-shareable)`.
    pub fn get_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<ProjectSettings, Error> {
        self.ensure_row(project_id, repo_dir)?;
        self.migrate_legacy_project_settings(project_id, repo_dir)?;
        let Some((base_json, shareable_json, _)) = self.read_project_row(project_id)? else {
            return Err(Error::Internal(
                "project settings row vanished after ensure".into(),
            ));
        };

        // Read-time legacy `remote` promotion (reference: `baseRemote ?? remote`)
        // — covers hand-written/older rows carrying a `remote` field.
        let base_json_value: serde_json::Value =
            parse_or_default(&base_json, "base_project_settings_json");
        let mut base: BaseProjectSettings =
            serde_json::from_value(base_json_value.clone()).unwrap_or_default();
        if base.base_remote.is_none() {
            if let Some(remote) = base_json_value.get("remote").and_then(|v| v.as_str()) {
                base.base_remote = Some(remote.to_string());
            }
        }
        let db_shareable: ShareableProjectSettings =
            parse_or_default(&shareable_json, "shareable_project_settings_json");
        let file_shareable = self.read_ade_json(repo_dir).unwrap_or_default();
        let shareable = merge_shareable(&[&default_shareable(), &file_shareable, &db_shareable]);

        // E1-05: a stored-invalid worktree directory falls back to the
        // default on read (reference: "stored invalid value falls back to
        // default on read"). Validate with the home dir; if it no longer
        // normalizes (e.g. a `~` entry after a home change), drop it.
        let worktree_directory = base.worktree_directory.and_then(|wd| {
            crate::settings::worktree_directory::normalize_worktree_directory(
                &wd,
                crate::settings::worktree_directory::home_dir().as_deref(),
            )
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
        });

        Ok(ProjectSettings {
            worktree_directory,
            default_branch: base.default_branch,
            base_remote: base.base_remote,
            push_remote: base.push_remote,
            github_account_id: base.github_account_id,
            tmux: base.tmux,
            auto_run_setup_script_on_task_creation: base.auto_run_setup_script_on_task_creation,
            auto_run_run_script_on_task_creation: base.auto_run_run_script_on_task_creation,
            task_startup_command: base.task_startup_command,
            workspace_provider: base.workspace_provider,
            preserve_patterns: shareable.preserve_patterns,
            shell_setup: shareable.shell_setup,
            scripts: shareable.scripts,
        })
    }

    /// Stores the given settings as local values: base fields go to
    /// `base_project_settings_json`, shareable fields to
    /// `shareable_project_settings_json` (which override `.fartCode.json`).
    /// The file itself is only touched by `share_with_team`.
    ///
    /// **Full-replace semantics** (matches the reference `update()`): both
    /// blobs are overwritten with what you send. Callers should pass the
    /// complete effective settings (e.g. read-modify-write); sending a
    /// partial object will clear the fields you omit.
    pub fn update_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
        settings: &ProjectSettings,
    ) -> Result<(), Error> {
        if let Some(wp) = &settings.workspace_provider {
            if wp.r#type != "script" {
                return Err(Error::InvalidSettingValue {
                    key: "workspaceProvider".into(),
                    reason: format!("unsupported provider type '{}'", wp.r#type),
                });
            }
        }
        self.ensure_row(project_id, repo_dir)?;
        let base = settings.base();
        // E1-05: store only the shareable DELTA vs the repo's .fartCode.json.
        // The panel sends the effective (merged) settings; fields that still
        // match the file must stay None in the DB so the file keeps winning
        // on read (otherwise a teammate's later .fartCode.json edit is shadowed).
        let file_shareable = self.read_ade_json(repo_dir).unwrap_or_default();
        let incoming = shareable_without_empty_scripts(&settings.shareable());
        let shareable = ShareableProjectSettings {
            preserve_patterns: delta_opt(
                &incoming.preserve_patterns,
                &file_shareable.preserve_patterns,
            ),
            shell_setup: delta_opt(&incoming.shell_setup, &file_shareable.shell_setup),
            scripts: delta_opt(&incoming.scripts, &file_shareable.scripts),
        };
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE project_settings
                 SET base_project_settings_json = ?1,
                     shareable_project_settings_json = ?2,
                     updated_at = datetime('now')
                 WHERE project_id = ?3",
                rusqlite::params![compact_json(&base)?, compact_json(&shareable)?, project_id],
            )?;
            Ok(())
        })
    }

    /// Runs the legacy v1.1.15-era `.emdash.json` migration once (guarded by
    /// `legacy_config_migrated_at`): base fields (incl. `remote` →
    /// `baseRemote` promotion and bare-branch remote prefixing) move into
    /// `base_project_settings_json`; shareable fields merge into the
    /// shareable blob. Shareable merging is unconditional in Phase 0 — the
    /// reference gates it on the file not being cleanly tracked by git, which
    /// needs `fartcode-git` (see note in code).
    ///
    /// The single marker covers both sides (base + shareable); the reference
    /// uses a second in-JSON marker for the shareable side. Because the
    /// migration runs on first access and marks itself done even without a
    /// legacy file, a `.emdash.json` dropped in *after* the first read is not
    /// migrated — parity with the reference.
    pub fn migrate_legacy_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<(), Error> {
        let Some((base_json, shareable_json, migrated_at)) = self.read_project_row(project_id)?
        else {
            self.ensure_row(project_id, repo_dir)?;
            return Ok(());
        };
        if migrated_at.is_some() {
            return Ok(());
        }
        let mut base: serde_json::Value =
            parse_or_default(&base_json, "base_project_settings_json");
        let mut shareable: serde_json::Value =
            parse_or_default(&shareable_json, "shareable_project_settings_json");
        // Corrupt-but-parseable blobs (e.g. `[1,2,3]`) must not panic the
        // IndexMut assignments below (reference `readJson` maps non-objects to {}).
        if !base.is_object() {
            base = serde_json::Value::Object(Default::default());
        }
        if !shareable.is_object() {
            shareable = serde_json::Value::Object(Default::default());
        }

        let legacy = self.read_legacy_config(repo_dir);
        if let Some(legacy) = legacy {
            let legacy_remote = legacy.remote.as_ref();
            let legacy_base_remote = legacy.base_remote.as_ref();
            if let Some(wd) = &legacy.worktree_directory {
                base["worktreeDirectory"] = serde_json::json!(wd);
            }
            // Legacy `remote` / `baseRemote` overwrite the seeded baseRemote
            // (reference migration: `next.baseRemote = legacy.remote`).
            if let Some(remote) = legacy_remote {
                base["baseRemote"] = serde_json::json!(remote);
            }
            if let Some(br) = legacy_base_remote {
                base["baseRemote"] = serde_json::json!(br);
            }
            if let Some(pr) = &legacy.push_remote {
                base["pushRemote"] = serde_json::json!(pr);
            }
            if let Some(db) = &legacy.default_branch {
                let remote = legacy_base_remote
                    .map(String::as_str)
                    .or_else(|| legacy_remote.map(String::as_str))
                    .or_else(|| base.get("baseRemote").and_then(|v| v.as_str()));
                let value = if !db.contains('/') {
                    remote
                        .map(|r| format!("{r}/{db}"))
                        .unwrap_or_else(|| db.clone())
                } else {
                    db.clone()
                };
                base["defaultBranch"] = serde_json::json!(value);
            }
            if let Some(t) = legacy.tmux {
                base["tmux"] = serde_json::json!(t);
            }
            if let Some(wp) = &legacy.workspace_provider {
                base["workspaceProvider"] = serde_json::json!(wp);
            }
            // Shareable: merge only fields the file defines.
            let db_shareable: ShareableProjectSettings =
                serde_json::from_value(shareable.clone()).unwrap_or_default();
            let file_shareable = ShareableProjectSettings {
                preserve_patterns: legacy.preserve_patterns.clone(),
                shell_setup: legacy.shell_setup.clone(),
                scripts: legacy.scripts.clone(),
            };
            let merged = merge_shareable(&[&db_shareable, &file_shareable]);
            shareable = serde_json::from_str(&compact_json(&merged)?)?;
        }

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE project_settings
                 SET base_project_settings_json = ?1,
                     shareable_project_settings_json = ?2,
                     legacy_config_migrated_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE project_id = ?3",
                rusqlite::params![base.to_string(), shareable.to_string(), project_id],
            )?;
            Ok(())
        })
    }

    /// Which source wins each shareable key under the effective merge
    /// `defaults < .fartCode.json < DB` (7c shared-vs-local tags):
    /// `"local"` = a DB override beats the file/defaults, `"shared"` = the
    /// repo `.fartCode.json` value wins, `"default"` = neither defines it.
    /// Keys are the wire names (`preservePatterns`, `shellSetup`, `scripts`).
    ///
    /// Seeding materializes the default preserve patterns into the DB blob
    /// (see `seed_project_settings`); a DB value that still EQUALS the
    /// default with no file behind it therefore reports `"default"`, not
    /// `"local"` — a fresh project carries no override tags.
    pub fn shareable_provenance(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<std::collections::BTreeMap<String, String>, Error> {
        self.ensure_row(project_id, repo_dir)?;
        let Some((_, shareable_json, _)) = self.read_project_row(project_id)? else {
            return Err(Error::Internal(
                "project settings row vanished after ensure".into(),
            ));
        };
        let local: ShareableProjectSettings =
            parse_or_default(&shareable_json, "shareable_project_settings_json");
        let file = self.read_ade_json(repo_dir).unwrap_or_default();
        let defaults = default_shareable();

        fn source<T: PartialEq>(
            local: &Option<T>,
            file: &Option<T>,
            default: &Option<T>,
        ) -> &'static str {
            match (local, file) {
                // A local value identical to the default with no file
                // underneath is the seed materialization, not an override.
                (Some(l), None) if Some(l) == default.as_ref() => "default",
                (Some(_), _) => "local",
                (None, Some(_)) => "shared",
                (None, None) => "default",
            }
        }

        let mut out = std::collections::BTreeMap::new();
        out.insert(
            "preservePatterns".to_string(),
            source(
                &local.preserve_patterns,
                &file.preserve_patterns,
                &defaults.preserve_patterns,
            )
            .to_string(),
        );
        out.insert(
            "shellSetup".to_string(),
            source(&local.shell_setup, &file.shell_setup, &defaults.shell_setup).to_string(),
        );
        out.insert(
            "scripts".to_string(),
            source(&local.scripts, &file.scripts, &defaults.scripts).to_string(),
        );
        Ok(out)
    }

    // -- Internal helpers ---------------------------------------------------

    fn ensure_row(&self, project_id: &str, repo_dir: &Path) -> Result<(), Error> {
        let exists: bool = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM project_settings WHERE project_id = ?1",
                    [project_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })?;
        if !exists {
            self.seed_project_settings(project_id, repo_dir)?;
        }
        Ok(())
    }

    fn read_project_row(
        &self,
        project_id: &str,
    ) -> Result<Option<(String, String, Option<String>)>, Error> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT base_project_settings_json, shareable_project_settings_json,
                            legacy_config_migrated_at
                     FROM project_settings WHERE project_id = ?1",
                    [project_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?)
        })
    }

    /// Reads and parses the repo `.fartCode.json` (shareable schema). Missing or
    /// unparseable files are treated as absent (warn + `None`), matching the
    /// reference's fallback behavior.
    fn read_ade_json(&self, repo_dir: &Path) -> Option<ShareableProjectSettings> {
        let content = std::fs::read_to_string(repo_dir.join(PROJECT_CONFIG_FILE)).ok()?;
        match serde_json::from_str(&content) {
            Ok(settings) => Some(settings),
            Err(e) => {
                tracing::warn!(
                    path = %repo_dir.join(PROJECT_CONFIG_FILE).display(),
                    error = %e,
                    "unparseable .fartCode.json — falling back to project settings"
                );
                None
            }
        }
    }

    fn read_legacy_config(&self, repo_dir: &Path) -> Option<registry::LegacyProjectConfig> {
        let path = repo_dir.join(LEGACY_CONFIG_FILE);
        let content = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str(&content) {
            Ok(config) => Some(config),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "unparseable legacy config — nothing to migrate"
                );
                None
            }
        }
    }
}

impl SettingsStore for DbSettingsStore {
    fn get_json(&self, key: &str) -> Result<serde_json::Value, Error> {
        let defaults =
            registry::default_value(key).ok_or_else(|| Error::InvalidSettingKey(key.into()))?;
        let stored: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()?)
        })?;
        match stored {
            None => Ok(defaults),
            Some(raw) => match serde_json::from_str(&raw) {
                Err(e) => {
                    tracing::warn!(key, error = %e, "unparseable app_settings row — using defaults");
                    Ok(defaults)
                }
                Ok(serde_json::Value::Null) => Ok(defaults),
                Ok(raw_value) => {
                    if defaults.is_object() && raw_value.is_object() {
                        let mut merged = defaults;
                        merge_deep(&mut merged, &raw_value);
                        Ok(merged)
                    } else {
                        Ok(raw_value)
                    }
                }
            },
        }
    }

    fn set_json(&self, key: &str, value: serde_json::Value) -> Result<(), Error> {
        // Validation by round-trip: unknown keys are stripped (reference zod
        // `.parse`), so deltas never carry junk fields.
        let canonical = registry::canonical_value(key, &value)?;
        let defaults =
            registry::default_value(key).ok_or_else(|| Error::InvalidSettingKey(key.into()))?;
        let delta = compute_delta(&defaults, &canonical);
        let empty = delta.as_object().is_some_and(|m| m.is_empty());
        self.with_conn(|conn| {
            if empty {
                conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])?;
            } else {
                conn.execute(
                    "INSERT INTO app_settings (key, value, updated_at)
                     VALUES (?1, ?2, unixepoch())
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
                    rusqlite::params![key, delta.to_string()],
                )?;
            }
            Ok(())
        })
    }

    fn reset(&self, project_id: Option<&str>) -> Result<(), Error> {
        self.with_conn(|conn| {
            match project_id {
                None => conn.execute("DELETE FROM app_settings", [])?,
                Some(pid) => {
                    conn.execute("DELETE FROM project_settings WHERE project_id = ?1", [pid])?
                }
            };
            Ok(())
        })
    }

    fn share_with_team(&self, project_id: &str) -> Result<bool, Error> {
        let repo_path: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT path FROM projects WHERE id = ?1",
                    [project_id],
                    |row| row.get(0),
                )
                .optional()?)
        })?;
        let Some(repo_path) = repo_path else {
            return Err(Error::ProjectNotFound(project_id.into()));
        };

        let Some((_, shareable_json, _)) = self.read_project_row(project_id)? else {
            return Ok(false);
        };
        let shareable: ShareableProjectSettings =
            parse_or_default(&shareable_json, "shareable_project_settings_json");
        if shareable == ShareableProjectSettings::default() {
            return Ok(false); // nothing local to share
        }

        // Patch the repo's .fartCode.json with the local values. A missing file is
        // created; an existing-but-unparseable file is left untouched (error),
        // matching the reference (parse failure -> writeConfigFailed).
        let config_path = PathBuf::from(&repo_path).join(PROJECT_CONFIG_FILE);
        let mut config: serde_json::Value = match std::fs::read_to_string(&config_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(value) => value,
                Err(e) => {
                    return Err(Error::Internal(format!(
                        "cannot share settings: {} is not valid JSON ({e})",
                        config_path.display()
                    )));
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::Value::Object(Default::default())
            }
            Err(e) => return Err(Error::Io(e)),
        };
        if !config.is_object() {
            config = serde_json::Value::Object(Default::default());
        }
        apply_shareable(&mut config, &shareable);
        let content = format!("{}\n", serde_json::to_string_pretty(&config)?);
        // Atomic write: temp file in the same dir + rename, so a crash
        // mid-write can't corrupt .fartCode.json (the read path treats an
        // unparseable file as absent, silently losing shareable settings).
        let tmp = config_path.with_extension("fartCode.json.tmp");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &config_path)?;

        // Clear the moved fields from the DB.
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE project_settings SET shareable_project_settings_json = '{}' WHERE project_id = ?1",
                [project_id],
            )?;
            Ok(())
        })?;
        Ok(true)
    }

    fn seed_project_settings(&self, project_id: &str, repo_dir: &Path) -> Result<(), Error> {
        self.seed_project_settings(project_id, repo_dir)
    }

    fn get_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<ProjectSettings, Error> {
        self.get_project_settings(project_id, repo_dir)
    }

    fn update_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
        settings: &ProjectSettings,
    ) -> Result<(), Error> {
        self.update_project_settings(project_id, repo_dir, settings)
    }

    fn migrate_legacy_project_settings(
        &self,
        project_id: &str,
        repo_dir: &Path,
    ) -> Result<(), Error> {
        self.migrate_legacy_project_settings(project_id, repo_dir)
    }
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

/// Shareable delta for `update_project_settings`: a field that still equals
/// the `.fartCode.json` value stays `None` in the DB so the file keeps winning on
/// read; only genuine overrides (or clears) are stored.
fn delta_opt<T: PartialEq + Clone>(incoming: &Option<T>, file: &Option<T>) -> Option<T> {
    if incoming == file {
        None
    } else {
        incoming.clone()
    }
}

/// Recursively merges `overrides` into `base` (objects merge per-key, later
/// wins; anything else is replaced). Mirrors the reference `mergeDeep`.
fn merge_deep(base: &mut serde_json::Value, overrides: &serde_json::Value) {
    match (base, overrides) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(override_map)) => {
            for (key, value) in override_map {
                if let Some(existing) = base_map.get_mut(key) {
                    if existing.is_object() && value.is_object() {
                        merge_deep(existing, value);
                    } else {
                        base_map.insert(key.clone(), value.clone());
                    }
                } else {
                    base_map.insert(key.clone(), value.clone());
                }
            }
        }
        (base, overrides) => *base = overrides.clone(),
    }
}

/// Delta vs defaults: for objects, only top-level fields differing from the
/// default (deep-equal comparison); for scalars, an empty object when equal
/// (so the row gets deleted) or the whole value.
fn compute_delta(defaults: &serde_json::Value, value: &serde_json::Value) -> serde_json::Value {
    if let (serde_json::Value::Object(default_map), serde_json::Value::Object(value_map)) =
        (defaults, value)
    {
        let mut delta = serde_json::Map::new();
        for (key, val) in value_map {
            if default_map.get(key) != Some(val) {
                delta.insert(key.clone(), val.clone());
            }
        }
        serde_json::Value::Object(delta)
    } else if defaults == value {
        serde_json::Value::Object(Default::default())
    } else {
        value.clone()
    }
}

/// Later sources overwrite earlier ones per shareable field (reference
/// `mergeShareableProjectSettings`). `preservePatterns` is filtered of the
/// project's own config file at merge time.
fn merge_shareable(sources: &[&ShareableProjectSettings]) -> ShareableProjectSettings {
    let mut out = ShareableProjectSettings::default();
    for source in sources {
        if let Some(patterns) = &source.preserve_patterns {
            out.preserve_patterns = Some(registry::filter_config_file(patterns));
        }
        if let Some(shell) = &source.shell_setup {
            out.shell_setup = Some(shell.clone());
        }
        if let Some(scripts) = &source.scripts {
            out.scripts = Some(scripts.clone());
        }
    }
    out
}

fn default_shareable() -> ShareableProjectSettings {
    ShareableProjectSettings {
        preserve_patterns: Some(
            DEFAULT_PRESERVE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ),
        ..Default::default()
    }
}

/// An all-`None` `scripts` object is treated as absent (reference deletes the
/// object when every field is undefined).
fn shareable_without_empty_scripts(
    shareable: &ShareableProjectSettings,
) -> ShareableProjectSettings {
    let scripts = shareable
        .scripts
        .as_ref()
        .filter(|s| s.setup.is_some() || s.run.is_some() || s.teardown.is_some())
        .cloned();
    ShareableProjectSettings {
        scripts,
        ..shareable.clone()
    }
}

/// Serializes `value`, omitting `None` fields (reference `compactUndefined`).
fn compact_json<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut json = serde_json::to_value(value)?;
    strip_nulls(&mut json);
    Ok(json.to_string())
}

fn strip_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_nulls(v);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                strip_nulls(v);
            }
        }
        _ => {}
    }
}

fn parse_or_default<T: Default + serde::de::DeserializeOwned>(raw: &str, column: &str) -> T {
    serde_json::from_str(raw).unwrap_or_else(|e| {
        tracing::warn!(column, error = %e, "unparseable project settings blob — using defaults");
        T::default()
    })
}

/// Writes the defined shareable fields into a `.fartCode.json` config object.
fn apply_shareable(config: &mut serde_json::Value, shareable: &ShareableProjectSettings) {
    let obj = config
        .as_object_mut()
        .expect("config is guaranteed an object");
    if let Some(patterns) = &shareable.preserve_patterns {
        obj.insert(
            "preservePatterns".into(),
            serde_json::json!(registry::filter_config_file(patterns)),
        );
    }
    if let Some(shell) = &shareable.shell_setup {
        obj.insert("shellSetup".into(), serde_json::json!(shell));
    }
    if let Some(scripts) = &shareable.scripts {
        let entry = obj
            .entry("scripts".to_string())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(map) = entry.as_object_mut() {
            if let Some(v) = &scripts.setup {
                map.insert("setup".into(), serde_json::json!(v));
            }
            if let Some(v) = &scripts.run {
                map.insert("run".into(), serde_json::json!(v));
            }
            if let Some(v) = &scripts.teardown {
                map.insert("teardown".into(), serde_json::json!(v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    fn fixture() -> (tempfile::TempDir, DbSettingsStore, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let db = SqliteDb::init(Some(tmp.path().join("test.db").to_str().unwrap())).unwrap();
        let store = DbSettingsStore::new(db);
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        // project_settings FKs projects; share_with_team also needs the
        // path lookup.
        store
            .db
            .conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'fx', ?1)",
                [repo.to_str().unwrap()],
            )
            .unwrap();
        (tmp, store, repo)
    }

    #[test]
    fn provenance_fresh_project_is_all_default() {
        let (_tmp, store, repo) = fixture();
        let prov = store.shareable_provenance("p1", &repo).unwrap();
        // Seeding materializes default preservePatterns into the DB blob —
        // that must NOT read as a local override.
        assert_eq!(prov["preservePatterns"], "default");
        assert_eq!(prov["shellSetup"], "default");
        assert_eq!(prov["scripts"], "default");
    }

    #[test]
    fn provenance_file_values_are_shared_and_db_overrides_are_local() {
        let (_tmp, store, repo) = fixture();
        std::fs::write(
            repo.join(PROJECT_CONFIG_FILE),
            r#"{"shellSetup": "export A=1", "preservePatterns": [".env"]}"#,
        )
        .unwrap();
        let prov = store.shareable_provenance("p1", &repo).unwrap();
        assert_eq!(prov["shellSetup"], "shared");
        assert_eq!(prov["preservePatterns"], "shared");
        assert_eq!(prov["scripts"], "default");

        // A DB value differing from the file is a local override.
        let mut settings = store.get_project_settings("p1", &repo).unwrap();
        settings.shell_setup = Some("export A=2".into());
        store
            .update_project_settings("p1", &repo, &settings)
            .unwrap();
        let prov = store.shareable_provenance("p1", &repo).unwrap();
        assert_eq!(prov["shellSetup"], "local");
        // Unchanged-vs-file fields stay attributed to the file (delta write).
        assert_eq!(prov["preservePatterns"], "shared");
    }

    #[test]
    fn provenance_local_only_value_is_local_and_share_flips_it_shared() {
        let (_tmp, store, repo) = fixture();
        let mut settings = store.get_project_settings("p1", &repo).unwrap();
        settings.shell_setup = Some("export B=1".into());
        store
            .update_project_settings("p1", &repo, &settings)
            .unwrap();
        let prov = store.shareable_provenance("p1", &repo).unwrap();
        assert_eq!(prov["shellSetup"], "local");

        // E1-02 share: local values move into .fartCode.json → "shared".
        assert!(store.share_with_team("p1").unwrap());
        let prov = store.shareable_provenance("p1", &repo).unwrap();
        assert_eq!(prov["shellSetup"], "shared");
    }
}
