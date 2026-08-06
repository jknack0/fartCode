//! Settings integration tests (ticket E1-02 acceptance criteria).
//!
//! All tests use `tempfile::tempdir()` — never the real app data path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::settings::{
    DbSettingsStore, ProjectGroup, ProjectSettings, SettingsStore, TerminalGroup, DEFAULT_AGENT,
    LOCAL_PROJECT, PROJECT, PROJECT_CONFIG_FILE, TERMINAL,
};

/// Fixture: temp DB + a repo dir + a seeded project row. Holds the `TempDir`
/// so the repo directory survives for the whole test.
struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    settings: DbSettingsStore,
    repo: PathBuf,
    project_id: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
        let settings = DbSettingsStore::new(db.clone());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        // A projects row is required for share_with_team (path lookup).
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'fixture', ?1)",
                [repo.to_str().unwrap()],
            )
            .unwrap();
        Self {
            _tmp: tmp,
            db,
            settings,
            repo,
            project_id: "p1".into(),
        }
    }

    fn ade_json(&self) -> PathBuf {
        self.repo.join(PROJECT_CONFIG_FILE)
    }

    fn write_ade_json(&self, content: &str) {
        std::fs::write(self.ade_json(), content).unwrap();
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Acceptance 1: updating a setting to its default removes the stored row
// ---------------------------------------------------------------------------

#[test]
fn test_updating_to_default_deletes_row() {
    let fx = Fixture::new();

    // Default: pushOnCreate = true. Override it.
    let group = ProjectGroup {
        push_on_create: false,
        ..Default::default()
    };
    fx.settings.set(&PROJECT, group).unwrap();

    let stored: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'project'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 1, "delta row must be stored");

    // Update back to the default → row deleted, value back to default.
    fx.settings.set(&PROJECT, ProjectGroup::default()).unwrap();
    let stored: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'project'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0, "setting to the default must delete the row");
    assert_eq!(
        fx.settings.get(&PROJECT).unwrap(),
        ProjectGroup::default(),
        "read must return defaults after row deletion"
    );
}

#[test]
fn test_scalar_setting_to_default_deletes_row() {
    let fx = Fixture::new();
    assert_eq!(fx.settings.get(&DEFAULT_AGENT).unwrap(), "claude");

    fx.settings.set(&DEFAULT_AGENT, "gemini".into()).unwrap();
    assert_eq!(fx.settings.get(&DEFAULT_AGENT).unwrap(), "gemini");

    fx.settings.set(&DEFAULT_AGENT, "claude".into()).unwrap();
    assert_eq!(fx.settings.get(&DEFAULT_AGENT).unwrap(), "claude");
    let stored: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'defaultAgent'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);
}

#[test]
fn test_delta_merges_with_defaults_on_read() {
    let fx = Fixture::new();
    // Only one field differs → only the delta is stored.
    let group = TerminalGroup {
        auto_copy_on_selection: true,
        ..Default::default()
    };
    fx.settings.set(&TERMINAL, group.clone()).unwrap();

    let raw: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'terminal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"autoCopyOnSelection": true}),
        "only the delta must be stored"
    );

    // Read deep-merges the delta with defaults.
    assert_eq!(fx.settings.get(&TERMINAL).unwrap(), group);
}

#[test]
fn test_invalid_value_rejected() {
    let fx = Fixture::new();
    // Unknown terminal shell must be rejected by the validation pass.
    let err = fx
        .settings
        .set_json(
            "terminal",
            serde_json::json!({"defaultShell": "definitely-not-a-shell"}),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("invalid setting value"),
        "unknown enum value must be rejected: {err}"
    );
    assert!(fx.settings.set_json("nope", serde_json::json!(1)).is_err());
}

#[test]
fn test_local_project_paths_default_to_home() {
    let fx = Fixture::new();
    let local = fx.settings.get(&LOCAL_PROJECT).unwrap();
    assert!(
        local.default_worktree_directory.ends_with("ade/worktrees"),
        "worktree dir must default under ~/ade: {}",
        local.default_worktree_directory
    );
}

// ---------------------------------------------------------------------------
// Acceptance 2: .ade.json honored; local value overrides it; clearing local
// falls back to the file
// ---------------------------------------------------------------------------

#[test]
fn test_ade_json_precedence_and_local_override() {
    let fx = Fixture::new();
    fx.write_ade_json(
        r#"{"preservePatterns": [".env", "custom.env"], "shellSetup": "source .envrc"}"#,
    );

    // File is honored.
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.preserve_patterns.as_deref(),
        Some(&[".env".to_string(), "custom.env".to_string()][..]),
        ".ade.json preservePatterns must be honored"
    );
    assert_eq!(settings.shell_setup.as_deref(), Some("source .envrc"));

    // Local (DB) value overrides the file.
    let local = ProjectSettings {
        preserve_patterns: Some(vec!["local.env".into()]),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &local)
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.preserve_patterns.as_deref(),
        Some(&["local.env".to_string()][..]),
        "local value must override .ade.json"
    );

    // Clearing the local value falls back to the file.
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &ProjectSettings::default())
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.preserve_patterns.as_deref(),
        Some(&[".env".to_string(), "custom.env".to_string()][..]),
        "clearing the local value must fall back to .ade.json"
    );
}

#[test]
fn test_unparseable_ade_json_falls_back() {
    let fx = Fixture::new();
    fx.write_ade_json("{not valid json");
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let patterns = settings.preserve_patterns.unwrap();
    assert_eq!(
        patterns,
        ade_core::settings::DEFAULT_PRESERVE_PATTERNS.to_vec(),
        "unparseable .ade.json must fall back to defaults"
    );
}

#[test]
fn test_task_startup_command_round_trip_and_not_shareable() {
    let fx = Fixture::new();

    // Unset → None (blank-shell behavior).
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(settings.task_startup_command, None);

    // Set → round-trips through the DB (base field).
    let local = ProjectSettings {
        task_startup_command: Some("omp".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &local)
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(settings.task_startup_command.as_deref(), Some("omp"));

    // Clearing restores None.
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &ProjectSettings::default())
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(settings.task_startup_command, None);

    // Base, not shareable: when the file IS written (a shareable field
    // forces it), taskStartupCommand must not leak into .ade.json.
    let local = ProjectSettings {
        task_startup_command: Some("omp --task".into()),
        shell_setup: Some("source .envrc".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &local)
        .unwrap();
    assert!(fx.settings.share_with_team(&fx.project_id).unwrap());
    let config = read_json(&fx.ade_json());
    assert_eq!(
        config["shellSetup"],
        serde_json::json!("source .envrc"),
        "shareable field must be written"
    );
    assert!(
        !config
            .as_object()
            .unwrap()
            .contains_key("taskStartupCommand"),
        "taskStartupCommand is a local preference and must not be shared"
    );
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(settings.task_startup_command.as_deref(), Some("omp --task"));
}

#[test]
fn test_ade_json_filtered_from_preserve_patterns() {
    let fx = Fixture::new();
    fx.write_ade_json(r#"{"preservePatterns": [".env", ".ade.json", "keep.txt"]}"#);
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let patterns = settings.preserve_patterns.unwrap();
    assert!(
        !patterns.iter().any(|p| p == ".ade.json"),
        "config file must be filtered: {patterns:?}"
    );
    assert_eq!(patterns.len(), 2);
}

// ---------------------------------------------------------------------------
// Acceptance 3: legacy v1.1.15-era config migrates once into base JSON
// ---------------------------------------------------------------------------

#[test]
fn test_legacy_migration_runs_once() {
    let fx = Fixture::new();
    // Seed the row first (as project creation would).
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    // v1.1.15-era .emdash.json with scripts + preserve patterns + remote.
    std::fs::write(
        fx.repo.join(".emdash.json"),
        r#"{
          "worktreeDirectory": "~/dev/wt",
          "remote": "upstream",
          "defaultBranch": "main",
          "tmux": true,
          "preservePatterns": [".env", "legacy.env"],
          "shellSetup": "echo legacy"
        }"#,
    )
    .unwrap();

    fx.settings
        .migrate_legacy_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    // Read normalizes (reference resolveAndValidateWorktreeDirectory expands
    // ~ and validates on read — E1-05).
    let home = std::env::var("HOME").unwrap();
    assert_eq!(
        settings.worktree_directory,
        Some(format!("{home}/dev/wt")),
        "legacy ~ expands on read"
    );
    assert_eq!(
        settings.base_remote.as_deref(),
        Some("upstream"),
        "legacy remote promotes to baseRemote"
    );
    assert_eq!(
        settings.default_branch,
        Some(ade_core::settings::DefaultBranch::Name(
            "upstream/main".into()
        )),
        "bare branch must be prefixed with the remote name"
    );
    assert_eq!(settings.tmux, Some(true));
    assert_eq!(
        settings.preserve_patterns.as_deref(),
        Some(&[".env".to_string(), "legacy.env".to_string()][..]),
        "legacy shareable fields must migrate"
    );

    // The marker is set → running again is a no-op.
    fx.settings
        .migrate_legacy_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let migrated_at: Option<String> = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT legacy_config_migrated_at FROM project_settings WHERE project_id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        migrated_at.is_some(),
        "legacy_config_migrated_at must be set"
    );

    // Changing the legacy file afterwards must NOT re-migrate.
    std::fs::write(fx.repo.join(".emdash.json"), r#"{"remote": "other"}"#).unwrap();
    fx.settings
        .migrate_legacy_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.base_remote.as_deref(),
        Some("upstream"),
        "migration must run only once"
    );
}

// ---------------------------------------------------------------------------
// Seeding + share_with_team
// ---------------------------------------------------------------------------

#[test]
fn test_seed_respects_existing_ade_json() {
    let fx = Fixture::new();
    fx.write_ade_json(r#"{"preservePatterns": ["file.env"]}"#);

    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    // The row's shareable blob must be empty (file already defines patterns).
    let raw: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT shareable_project_settings_json FROM project_settings WHERE project_id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        raw, "{}",
        "shareable blob must start empty when the file defines patterns"
    );

    // Base defaults seeded: main / origin / tmux from project settings.
    let raw: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT base_project_settings_json FROM project_settings WHERE project_id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let base: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(base["defaultBranch"], serde_json::json!("main"));
    assert_eq!(base["baseRemote"], serde_json::json!("origin"));
    assert_eq!(base["tmux"], serde_json::json!(false));
}

#[test]
fn test_seed_default_preserve_patterns_when_no_file() {
    let fx = Fixture::new();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let raw: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT shareable_project_settings_json FROM project_settings WHERE project_id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let shareable: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        shareable["preservePatterns"],
        serde_json::json!(ade_core::settings::DEFAULT_PRESERVE_PATTERNS)
    );
}

#[test]
fn test_share_with_team_moves_values_to_file() {
    let fx = Fixture::new();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    // Local values to share.
    let local = ProjectSettings {
        preserve_patterns: Some(vec!["team.env".into()]),
        shell_setup: Some("source .envrc".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &local)
        .unwrap();

    fx.settings.share_with_team(&fx.project_id).unwrap();

    // File now holds them, pretty-printed.
    let config = read_json(&fx.ade_json());
    assert_eq!(config["preservePatterns"], serde_json::json!(["team.env"]));
    assert_eq!(config["shellSetup"], serde_json::json!("source .envrc"));
    let content = std::fs::read_to_string(fx.ade_json()).unwrap();
    assert!(content.ends_with('\n'), ".ade.json must end with a newline");

    // DB shareable cleared → effective falls back to the file.
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.preserve_patterns.as_deref(),
        Some(&["team.env".to_string()][..]),
        "effective value must survive the move (now sourced from the file)"
    );
}

#[test]
fn test_share_with_team_unknown_project_errors() {
    let fx = Fixture::new();
    let err = fx.settings.share_with_team("no-such-project").unwrap_err();
    assert!(err.to_string().contains("project not found"));
}

#[test]
fn test_reset_app_level_and_project_level() {
    let fx = Fixture::new();
    fx.settings.set(&DEFAULT_AGENT, "gemini".into()).unwrap();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    // Project-level reset clears the project row (reseeds on next read).
    fx.settings.reset(Some(&fx.project_id)).unwrap();
    let stored: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM project_settings WHERE project_id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);
    // App-level reset clears all app_settings rows.
    fx.settings.reset(None).unwrap();
    assert_eq!(fx.settings.get(&DEFAULT_AGENT).unwrap(), "claude");
    let stored: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, 0);
}

#[test]
fn test_trait_object_is_usable() {
    let fx = Fixture::new();
    let store: Arc<dyn SettingsStore> = Arc::new(fx.settings);
    store
        .set_json("defaultAgent", serde_json::json!("gemini"))
        .unwrap();
    assert_eq!(
        store.get_json("defaultAgent").unwrap(),
        serde_json::json!("gemini")
    );
}

// A custom value type round-trips through the typed-key mechanism (the
// blanket `SettingValue` impl makes the registry open to new types).
#[test]
fn test_typed_roundtrip() {
    let fx = Fixture::new();
    let mut terminal = fx.settings.get(&TERMINAL).unwrap();
    terminal.mac_option_is_meta = true;
    fx.settings.set(&TERMINAL, terminal.clone()).unwrap();
    assert_eq!(fx.settings.get(&TERMINAL).unwrap(), terminal);
}

// ---------------------------------------------------------------------------
// Review-fix regressions
// ---------------------------------------------------------------------------

#[test]
fn test_share_with_team_preserves_unparseable_ade_json() {
    let fx = Fixture::new();
    fx.write_ade_json("{not valid json");

    let local = ProjectSettings {
        shell_setup: Some("source .envrc".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &local)
        .unwrap();

    // share_with_team must error instead of clobbering the corrupt file.
    let err = fx.settings.share_with_team(&fx.project_id).unwrap_err();
    assert!(
        err.to_string().contains("not valid JSON"),
        "unparseable .ade.json must abort sharing: {err}"
    );
    let content = std::fs::read_to_string(fx.ade_json()).unwrap();
    assert_eq!(content, "{not valid json", "file must be left untouched");
}

#[test]
fn test_legacy_migration_survives_corrupt_base_blob() {
    let fx = Fixture::new();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    // Corrupt-but-parseable base JSON: an array must not panic migration.
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE project_settings SET base_project_settings_json = '[1,2,3]' WHERE project_id = 'p1'",
            [],
        )
        .unwrap();
    std::fs::write(fx.repo.join(".emdash.json"), r#"{"remote": "upstream"}"#).unwrap();

    // Must not panic; effective base falls back to defaults.
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.base_remote.as_deref(),
        Some("upstream"),
        "legacy remote still migrates"
    );
}

#[test]
fn test_set_json_strips_unknown_keys() {
    let fx = Fixture::new();
    fx.settings
        .set_json(
            "terminal",
            serde_json::json!({"bogusKey": 123, "autoCopyOnSelection": true}),
        )
        .unwrap();

    // The junk key must not persist; the delta only holds canonical fields.
    let raw: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'terminal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        stored,
        serde_json::json!({"autoCopyOnSelection": true}),
        "unknown keys must be stripped from the stored delta"
    );
    let effective = fx.settings.get_json("terminal").unwrap();
    assert!(
        effective.get("bogusKey").is_none(),
        "junk must not surface on read"
    );

    // An entirely-junk value computes an empty delta -> row deleted.
    fx.settings
        .set_json("terminal", serde_json::json!({"bogusKey": 1}))
        .unwrap();
    let rows: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'terminal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0, "junk-only value must delete the row (empty delta)");
}

#[test]
fn test_read_time_remote_promotion() {
    let fx = Fixture::new();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    // Hand-written row carrying only the legacy `remote` field.
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE project_settings SET base_project_settings_json = '{\"remote\": \"fork\"}' WHERE project_id = 'p1'",
            [],
        )
        .unwrap();
    let settings = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        settings.base_remote.as_deref(),
        Some("fork"),
        "remote must promote on read"
    );
}

// ---------------------------------------------------------------------------
// E1-05: worktree-directory validation + read-time fallback
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_worktree_directory_falls_back_on_read() {
    let fx = Fixture::new();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    // Store an invalid (relative) value directly — the read must drop it.
    let settings = ProjectSettings {
        worktree_directory: Some("relative/path".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &settings)
        .unwrap();
    let read = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        read.worktree_directory, None,
        "invalid stored value falls back to default on read"
    );

    // Tilde values normalize on read.
    let settings = ProjectSettings {
        worktree_directory: Some("~/ade/wt".into()),
        ..Default::default()
    };
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &settings)
        .unwrap();
    let read = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    let home = std::env::var("HOME").unwrap();
    assert_eq!(
        read.worktree_directory,
        Some(format!("{home}/ade/wt")),
        "~ expands on read"
    );
}

#[test]
fn test_normalize_worktree_directory_rules() {
    use ade_core::settings::worktree_directory::normalize_worktree_directory;
    let home = "/Users/ade";
    assert!(normalize_worktree_directory("/abs", None).is_ok());
    assert!(normalize_worktree_directory("~/wt", Some(home)).is_ok());
    assert!(matches!(
        normalize_worktree_directory("rel/wt", None),
        Err(ade_core::Error::InvalidWorktreeDirectory(_))
    ));
    assert!(matches!(
        normalize_worktree_directory("~/wt", None),
        Err(ade_core::Error::InvalidWorktreeDirectory(_))
    ));
}

// ---------------------------------------------------------------------------
// E1-05: update stores only the delta vs .ade.json (file keeps precedence)
// ---------------------------------------------------------------------------

#[test]
fn test_update_stores_only_delta_vs_ade_json() {
    let fx = Fixture::new();
    // A repo that already carries shareable values when added.
    std::fs::write(
        fx.repo.join(".ade.json"),
        r#"{"preservePatterns": [".env"], "shellSetup": "source .envrc"}"#,
    )
    .unwrap();
    fx.settings
        .seed_project_settings(&fx.project_id, &fx.repo)
        .unwrap();

    // The panel sends the effective merged object; unchanged fields must NOT
    // be materialized into the DB (the file keeps winning on read).
    let effective = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &effective)
        .unwrap();
    let (_, shareable_json, _) = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT base_project_settings_json, shareable_project_settings_json, base_project_settings_json FROM project_settings WHERE project_id = 'p1'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .unwrap();
    let shareable: serde_json::Value = serde_json::from_str(&shareable_json).unwrap();
    assert_eq!(
        shareable.get("preservePatterns"),
        None,
        "unchanged preservePatterns must stay out of the DB: {shareable_json}"
    );
    assert_eq!(
        shareable.get("shellSetup"),
        None,
        "unchanged shellSetup must stay out of the DB: {shareable_json}"
    );

    // A genuine override IS stored (and beats the file on read).
    let mut changed = effective.clone();
    changed.preserve_patterns = Some(vec!["local.env".into()]);
    fx.settings
        .update_project_settings(&fx.project_id, &fx.repo, &changed)
        .unwrap();
    let read = fx
        .settings
        .get_project_settings(&fx.project_id, &fx.repo)
        .unwrap();
    assert_eq!(
        read.preserve_patterns.as_deref(),
        Some(&["local.env".to_string()][..]),
        "override wins over the file"
    );
}
