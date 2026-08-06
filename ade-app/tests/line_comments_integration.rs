//! E4-10 acceptance: `create_task_from_comment` end-to-end — a real repo +
//! project, a review comment, the §14 prompt template, and the
//! bidirectional comment↔task link.
//!
//! Runs against `App::init(":memory:")` with a tempdir repo; the project's
//! worktree_directory is redirected into a tempdir so provisioning never
//! touches the real home directory.

use std::path::Path;
use std::process::Command;

use ade_app_lib::app::App;
use ade_app_lib::commands::line_comments::create_task_from_comment_core;
use ade_app_lib::terminals::TerminalManager;
use ade_core::projects::ProjectStore;
use ade_core::settings::SettingsStore;

fn terminal_manager() -> TerminalManager<tauri::test::MockRuntime> {
    TerminalManager::new(tauri::test::mock_app().handle().clone())
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .output()
        .expect("git spawns");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-b", "main"]);
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/auth.rs"), "fn validate() {}\n").unwrap();
    git(tmp.path(), &["add", "."]);
    git(tmp.path(), &["commit", "-m", "init"]);
    tmp
}

#[test]
fn create_task_from_comment_links_and_builds_prompt() {
    let repo = repo_fixture();
    let worktrees_dir = tempfile::tempdir().unwrap();

    let app = App::init(Some(":memory:")).expect("app init");

    // Project over the fixture repo; worktrees inside the tempdir (the
    // pool path reads the app-level localProject setting).
    let project = app
        .projects
        .create_local(repo.path(), false)
        .expect("create project");
    let local: ade_core::settings::LocalProjectGroup =
        serde_json::from_value(app.settings.get_json("localProject").unwrap()).unwrap();
    app.settings
        .set_json(
            "localProject",
            serde_json::json!({
                "defaultProjectsDirectory": local.default_projects_directory,
                "defaultWorktreeDirectory": worktrees_dir.path().to_string_lossy(),
                "writeAgentConfigToGitIgnore": local.write_agent_config_to_git_ignore,
            }),
        )
        .expect("set localProject");

    // The reviewed task: manual rows (task + workspace pointing at the repo)
    // so the comment anchors to a real branch without a provision round.
    {
        let conn = app.db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status)
             VALUES ('task-reviewed', ?1, 'reviewed', 'in_progress')",
            [&project.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, type, kind, location, path)
             VALUES ('ws-reviewed', 'local', 'worktree', 'local', ?1)",
            [repo.path().to_string_lossy().as_ref()],
        )
        .unwrap();
        conn.execute(
            "UPDATE tasks SET workspace_id = 'ws-reviewed' WHERE id = 'task-reviewed'",
            [],
        )
        .unwrap();
    }

    // The review comment on lines 1-1 of src/auth.rs (after side).
    let comment = app
        .line_comments
        .add(ade_core::line_comments::AddLineCommentOptions {
            task_id: "task-reviewed".into(),
            file_path: "src/auth.rs".into(),
            line_number: 1,
            line_end: Some(1),
            source_side: ade_core::line_comments::SourceSide::After,
            line_content: Some("fn validate() {}".into()),
            content: "This should return Result".into(),
            created_by: None,
        })
        .expect("add comment");

    // §14 "Create Task".
    let result = create_task_from_comment_core(
        &app,
        &terminal_manager(),
        &project.id,
        "fix-validate",
        &comment.id,
        "fn validate() {}",
        Some("fn validate()"),
    )
    .expect("create task from comment");

    // The prompt follows the §14 template — file, branch (the repo is on
    // main), enclosing function, selection range, reviewer comment.
    assert!(result
        .prompt
        .starts_with("You are reviewing code in a git diff.\n"));
    assert!(result.prompt.contains("FILE: src/auth.rs"));
    assert!(result.prompt.contains("BRANCH: main"));
    assert!(result.prompt.contains("ENCLOSING FUNCTION: fn validate()"));
    assert!(result
        .prompt
        .contains("SELECTED CODE (line 1):\nfn validate() {}"));
    assert!(result
        .prompt
        .contains("COMMENT FROM REVIEWER:\nThis should return Result"));

    // Bidirectional link (§14): comment → task AND task → comment.
    let comment = app.line_comments.get(&comment.id).unwrap().unwrap();
    assert_eq!(
        comment.linked_task_id.as_deref(),
        Some(result.task.id.as_str())
    );
    {
        let conn = app.db.conn().lock().unwrap();
        let source: Option<String> = conn
            .query_row(
                "SELECT source_comment_id FROM tasks WHERE id = ?1",
                [&result.task.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source.as_deref(), Some(comment.id.as_str()));
    }

    // The new task is provisioned on its own branch in the tempdir
    // worktrees, and its initial conversation carries the prompt.
    assert!(result.task.workspace_id.is_some());
    let ws_path: String = {
        let conn = app.db.conn().lock().unwrap();
        conn.query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [result.task.workspace_id.as_deref().unwrap()],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert!(
        ws_path.starts_with(worktrees_dir.path().to_string_lossy().as_ref()),
        "worktree escapes the tempdir: {ws_path}"
    );
    assert!(Path::new(&ws_path).exists());

    let config: serde_json::Value = {
        let conn = app.db.conn().lock().unwrap();
        let raw: String = conn
            .query_row(
                "SELECT config FROM conversations WHERE task_id = ?1
                  AND is_initial_conversation = 1",
                [&result.task.id],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    };
    // conversations.config carries build_config() output directly:
    // {version:"1", type:"pty", initialPrompt}.
    assert_eq!(config["version"], "1");
    assert_eq!(config["initialPrompt"], result.prompt);
}

#[test]
fn create_task_from_comment_missing_comment_errors() {
    let repo = repo_fixture();
    let app = App::init(Some(":memory:")).expect("app init");
    let project = app
        .projects
        .create_local(repo.path(), false)
        .expect("create project");

    let err = create_task_from_comment_core(
        &app,
        &terminal_manager(),
        &project.id,
        "x",
        "lc_missing",
        "code",
        None,
    )
    .expect_err("must fail");
    assert!(err.contains("line comment not found"));
}
