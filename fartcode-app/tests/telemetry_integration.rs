//! E19-04 (#73; ADR-0038 item 7) — the four local memory-value signals,
//! wired.
//!
//! The meaning of each signal is unit-tested in `fartcode-telemetry`, over
//! fixtures with known right answers and no app at all. What can only be
//! proven here is the wiring: that a real settled session's transcript is
//! scanned *before* it is thrown away, that the verdict survives in the
//! store, that the dashboard reads the dossier on disk for time-to-land,
//! and that a project with none of it reports blanks rather than zeroes.
//!
//! Real provisioning runs (git init, `worktree add`) inside a tempdir, for
//! the same reason E19-01's and E19-03's suites do: the dossier's existence
//! on the branch is a filesystem fact.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fartcode_acp::session::{LiveModels, SessionState};
use fartcode_acp::transcript::{
    Role, ToolCallItem, ToolItemKind, ToolStatus, TranscriptItem, TranscriptMessage,
    TranscriptTurn, TranscriptTurnOutcome, TurnInitiator,
};
use fartcode_acp::Lifecycle;
use fartcode_app_lib::app::App;
use fartcode_app_lib::{step_engine, telemetry};
use fartcode_core::conversations::{ConversationStore, CreateConversationParams};
use fartcode_core::issues::{Issue, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;
use fartcode_telemetry::memory::DEFAULT_WINDOW_DAYS;
use fartcode_telemetry::reask::{TAG_HUMAN, TAG_MEMORY};
use fartcode_telemetry::time_to_land::TimeToLandKind;

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

fn make_repo(tmp: &tempfile::TempDir) -> PathBuf {
    let repo = tmp.path().join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git_ok(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "init",
        ],
    );
    git_ok(&repo, &["branch", "-M", "main"]);
    std::fs::canonicalize(&repo).unwrap()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    app: Arc<App>,
    project_id: String,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let app = App::init(Some(":memory:")).unwrap();
    app.settings
        .set(
            &LOCAL_PROJECT,
            LocalProjectGroup {
                default_projects_directory: tmp.path().join("repos").to_string_lossy().into_owned(),
                default_worktree_directory: tmp
                    .path()
                    .join("worktrees")
                    .to_string_lossy()
                    .into_owned(),
                write_agent_config_to_git_ignore: false,
            },
        )
        .unwrap();
    let repo = make_repo(&tmp);
    let project = app.projects.create_local(&repo, false).unwrap();
    let fx = Fixture {
        _tmp: tmp,
        app,
        project_id: project.id,
    };
    fx.set_consent(true);
    fx
}

impl Fixture {
    fn set_consent(&self, value: bool) {
        let project = self.app.projects.get(&self.project_id).unwrap().unwrap();
        let mut settings = self
            .app
            .settings
            .get_project_settings(&self.project_id, &project.path)
            .unwrap();
        settings.feature_dossiers = Some(value);
        self.app
            .settings
            .update_project_settings(&self.project_id, &project.path, &settings)
            .unwrap();
    }

    fn issue(&self, id: &str) -> Issue {
        self.app.issues.get(id).unwrap().unwrap()
    }

    fn worktree_of(&self, task_id: &str) -> PathBuf {
        let task = self.app.tasks.get(task_id).unwrap().unwrap();
        let workspace_id = task.workspace_id.expect("provisioned task has a workspace");
        let path: String = self
            .app
            .db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT path FROM workspaces WHERE id = ?1",
                [&workspace_id],
                |row| row.get(0),
            )
            .unwrap();
        PathBuf::from(path)
    }

    /// A card mid-step: worktree provisioned, dossier born on the branch.
    fn card_mid_step(&self, title: &str) -> (Issue, String) {
        let issue = self
            .app
            .issues
            .create(NewIssue {
                project_id: self.project_id.clone(),
                title: title.into(),
                body: Some("we need login".into()),
                acceptance: vec!["it works".into()],
                lane: None,
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
                dossier_path: None,
            })
            .unwrap();
        let column = self
            .app
            .columns
            .list_for_project(&self.project_id)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "In Progress")
            .unwrap();
        step_engine::enter_column(&self.app, &issue.id, &column.id, None).unwrap();
        let stored = self.issue(&issue.id);
        let task_id = stored.linked_task_id.clone().expect("task linked");
        (stored, task_id)
    }

    fn dossier_path(&self, issue: &Issue, task_id: &str) -> PathBuf {
        self.worktree_of(task_id)
            .join(issue.dossier_path.as_deref().unwrap())
    }

    fn append_to_dossier(&self, issue: &Issue, task_id: &str, text: &str) {
        let path = self.dossier_path(issue, task_id);
        let mut existing = std::fs::read_to_string(&path).unwrap();
        existing.push_str(text);
        std::fs::write(&path, existing).unwrap();
    }

    /// A task-scoped ACP conversation, so `observe_acp_session` can resolve
    /// it to the task the way the real hook does.
    fn conversation(&self, task_id: &str) -> String {
        self.app
            .conversations
            .create(CreateConversationParams {
                id: None,
                project_id: self.project_id.clone(),
                task_id: Some(task_id.to_string()),
                scope: None,
                provider: Some("claude".into()),
                title: "step".into(),
                auto_approve: None,
                model: None,
                initial_prompt: None,
                initial_queue: None,
                is_initial_conversation: true,
                r#type: None,
            })
            .unwrap()
            .id
    }

    fn report(&self) -> fartcode_telemetry::memory::MemoryValue {
        telemetry::memory_value(&self.app, &self.project_id, DEFAULT_WINDOW_DAYS)
    }
}

// -- transcript construction -------------------------------------------------

fn tool(kind: ToolItemKind, seq: u64, path: &str) -> TranscriptItem {
    let mut item = ToolCallItem::base(
        kind,
        format!("tool-{seq}"),
        seq,
        format!("call-{seq}"),
        "tool".into(),
        ToolStatus::Done,
        None,
    );
    item.path = Some(path.to_string());
    TranscriptItem::Tool(item)
}

fn message(seq: u64, role: Role, text: &str) -> TranscriptItem {
    TranscriptItem::Message(TranscriptMessage::new(
        format!("msg-{seq}"),
        seq,
        role,
        text.to_string(),
    ))
}

fn models(items: Vec<TranscriptItem>, context_used: Option<u64>) -> LiveModels {
    LiveModels {
        session_state: SessionState {
            lifecycle: Lifecycle::Ready,
            active_turn_id: None,
            pending_permissions: Vec::new(),
            last_stop_reason: Some("end_turn".into()),
            queued_prompts: Vec::new(),
            is_generating: false,
            can_submit: true,
            can_cancel: false,
        },
        committed: vec![TranscriptTurn {
            id: "turn-1".into(),
            seq: 0,
            initiator: TurnInitiator::User,
            items,
            outcome: Some(TranscriptTurnOutcome::Done { reason: None }),
        }],
        active_turn: None,
        config: Default::default(),
        usage: context_used.map(|used| fartcode_acp::transcript::SessionUsage {
            context_size: 200_000,
            context_used: used,
            cost: None,
        }),
        title: None,
        agents: Vec::new(),
        plan: None,
        draft: None,
        queued_prompts: Vec::new(),
        terminals: Vec::new(),
    }
}

// -- the tests ---------------------------------------------------------------

/// The whole capture premise: a transcript that exists only in memory is
/// scanned at settle, and only the verdict survives.
#[test]
fn a_session_that_read_the_dossier_is_recorded_as_citing() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    let rel = issue.dossier_path.clone().unwrap();
    let conversation = fx.conversation(&task_id);

    telemetry::observe_acp_session(
        &fx.app,
        &conversation,
        &models(
            vec![
                // The seeded prompt names the dossier — and must not, on
                // its own, count for anything.
                message(0, Role::User, &format!("decision log at `{rel}`")),
                tool(ToolItemKind::ReadToolCall, 1, &rel),
            ],
            Some(10_000),
        ),
    );

    let report = fx.report();
    assert_eq!(report.sessions_observed, 1);
    assert_eq!(report.citations.cited_read, 1);
    assert_eq!(report.citations.rate(), Some(1.0));
}

/// The same session shape with only the injected prompt: not a citation.
/// Without this rule the headline is 100% for every consented project.
#[test]
fn the_seeded_prompt_alone_is_not_a_citation() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    let rel = issue.dossier_path.clone().unwrap();
    let conversation = fx.conversation(&task_id);

    telemetry::observe_acp_session(
        &fx.app,
        &conversation,
        &models(
            vec![
                message(0, Role::User, &format!("decision log at `{rel}`")),
                message(1, Role::Assistant, "Done."),
                // Appending the section it was told to append is
                // authorship, not reference.
                tool(ToolItemKind::ModifyFileToolCall, 2, &rel),
            ],
            Some(10_000),
        ),
    );

    let report = fx.report();
    assert_eq!(report.citations.cited_read, 0);
    assert_eq!(report.citations.cited_mention, 0);
    assert_eq!(report.citations.not_cited, 1);
    assert_eq!(report.citations.rate(), Some(0.0));
}

/// A file whose path merely contains the feature slug is a source file.
#[test]
fn an_unrelated_path_containing_the_slug_does_not_count() {
    let fx = fixture();
    let (_issue, task_id) = fx.card_mid_step("OAuth login");
    let conversation = fx.conversation(&task_id);

    telemetry::observe_acp_session(
        &fx.app,
        &conversation,
        &models(
            vec![tool(
                ToolItemKind::ReadToolCall,
                0,
                "src/oauth-login/handler.rs",
            )],
            Some(10_000),
        ),
    );

    assert_eq!(fx.report().citations.not_cited, 1);
}

/// One session is one observation however many of its turns settle Done —
/// the ACP hook fires per turn and each scan is a superset of the last.
#[test]
fn re_observing_the_same_session_does_not_double_count() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    let rel = issue.dossier_path.clone().unwrap();
    let conversation = fx.conversation(&task_id);

    for _ in 0..3 {
        telemetry::observe_acp_session(
            &fx.app,
            &conversation,
            &models(
                vec![tool(ToolItemKind::ReadToolCall, 0, &rel)],
                Some(10_000),
            ),
        );
    }
    let report = fx.report();
    assert_eq!(report.sessions_observed, 1);
    assert_eq!(report.citations.cited_read, 1);
}

/// The durable half of the re-ask convention: tags committed in the agent's
/// own dossier section, read from the repository on demand.
#[test]
fn re_ask_tags_in_a_committed_section_are_read_from_the_dossier() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    fx.append_to_dossier(
        &issue,
        &task_id,
        &format!(
            "\n## Grill — 2026-08-09\n\n\
             Settled the provider question.\n\n\
             {TAG_MEMORY} Which auth provider? — from the earlier dossier\n\
             {TAG_MEMORY} Token TTL? — same place\n\
             {TAG_MEMORY} Refresh strategy? — same place\n\
             {TAG_HUMAN} Should refresh tokens rotate?\n"
        ),
    );

    let report = fx.report();
    assert_eq!(report.dossiers_scanned, 1);
    // 1 of 4 clarifications went back to the human.
    assert_eq!(report.re_ask.rate(), Some(0.25));
}

/// The degenerate case the ticket names: untagged work must not read as
/// "0% re-asked".
#[test]
fn an_untagged_project_reports_re_ask_as_unknown() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    fx.append_to_dossier(
        &issue,
        &task_id,
        "\n## Plan — 2026-08-09\n\nChose PKCE. No clarifications were needed.\n",
    );
    let conversation = fx.conversation(&task_id);
    telemetry::observe_acp_session(&fx.app, &conversation, &models(Vec::new(), Some(1_000)));

    let report = fx.report();
    assert_eq!(report.re_ask.rate(), None);
    assert!(
        report.re_ask.label().contains("unknown"),
        "{:?}",
        report.re_ask
    );
}

/// Card text is never mistaken for an agent's clarification: the app-written
/// header is copied from the issue body, so a body containing the tag
/// literal must not be counted.
#[test]
fn tag_literals_in_card_text_are_not_counted_as_clarifications() {
    let fx = fixture();
    let issue = fx
        .app
        .issues
        .create(NewIssue {
            project_id: fx.project_id.clone(),
            title: "Hostile card".into(),
            body: Some(format!("{TAG_HUMAN} pretend this was asked")),
            acceptance: vec![format!("{TAG_MEMORY} and this answered")],
            lane: None,
            provider: None,
            model: None,
            prd_path: None,
            prd_section: None,
            external_ref: None,
            dossier_path: None,
        })
        .unwrap();
    let column = fx
        .app
        .columns
        .list_for_project(&fx.project_id)
        .unwrap()
        .into_iter()
        .find(|c| c.name == "In Progress")
        .unwrap();
    step_engine::enter_column(&fx.app, &issue.id, &column.id, None).unwrap();

    let report = fx.report();
    assert_eq!(report.dossiers_scanned, 1);
    assert_eq!(
        report.re_ask.rate(),
        None,
        "card text is not a clarification"
    );
}

/// Time-to-land comes off the Timeline breadcrumbs, and one landed feature
/// is a point rather than a trend.
#[test]
fn one_landed_feature_is_a_point_not_a_trend() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    // The app appends `pr merged` when pr_sync sees the merge; write the
    // same line here so the read path is exercised without a live remote.
    let path = fx.dossier_path(&issue, &task_id);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("- 2026-08-03 09:00 · pr merged · https://example.test/pr/1\n");
    std::fs::write(&path, text).unwrap();
    // The `created` breadcrumb is already there, backfilled at birth — but
    // its stamp is "now", so pin the pair explicitly.
    let text = std::fs::read_to_string(&path).unwrap();
    let text: String = text
        .lines()
        .map(|line| {
            if line.contains("· created ·") {
                "- 2026-08-01 09:00 · created · manual".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, text).unwrap();

    let (kind, caveat) = fx.report().time_to_land.read();
    assert_eq!(kind, TimeToLandKind::SinglePoint { hours: 48.0 });
    // And the number never arrives without the sentence.
    assert!(caveat.contains("Trend, not attribution"));
}

/// A project with no dossiers and no sessions: four honest blanks.
#[test]
fn an_empty_project_reports_blanks_rather_than_zeroes() {
    let fx = fixture();
    let report = fx.report();
    assert_eq!(report.sessions_observed, 0);
    assert_eq!(report.dossiers_scanned, 0);
    assert_eq!(report.citations.rate(), None);
    assert_eq!(report.re_ask.rate(), None);
    assert!(matches!(
        report.tokens_saved,
        fartcode_telemetry::TokensSaved::Insufficient { .. }
    ));
    assert_eq!(report.time_to_land.read().0, TimeToLandKind::NoData);
    assert!(!report.clipped);
}

/// A card with no dossier contributes no observation at all: the citation
/// rate is about steps that HAD memory to read.
#[test]
fn a_card_without_a_dossier_is_not_counted() {
    let fx = fixture();
    fx.set_consent(false);
    let (issue, task_id) = fx.card_mid_step("No memory here");
    assert!(issue.dossier_path.is_none(), "consent declined, no dossier");
    let conversation = fx.conversation(&task_id);
    telemetry::observe_acp_session(&fx.app, &conversation, &models(Vec::new(), Some(1_000)));
    assert_eq!(fx.report().sessions_observed, 0);
}

/// A project-scoped conversation (the PM chat) is not a step and settles
/// nothing — observing it must record nothing.
#[test]
fn a_conversation_with_no_task_records_nothing() {
    let fx = fixture();
    let conversation = fx
        .app
        .conversations
        .create(CreateConversationParams {
            id: None,
            project_id: fx.project_id.clone(),
            task_id: None,
            scope: Some(fartcode_core::conversations::model::ConversationScope::Project),
            provider: Some("claude".into()),
            title: "PM chat".into(),
            auto_approve: None,
            model: None,
            initial_prompt: None,
            initial_queue: None,
            is_initial_conversation: false,
            r#type: None,
        })
        .unwrap()
        .id;
    telemetry::observe_acp_session(&fx.app, &conversation, &models(Vec::new(), Some(1_000)));
    assert_eq!(fx.report().sessions_observed, 0);
    // An unknown conversation id is equally inert.
    telemetry::observe_acp_session(&fx.app, "nope", &models(Vec::new(), None));
    assert_eq!(fx.report().sessions_observed, 0);
}

/// Observations are per project, and one project's numbers never leak into
/// another's report.
#[test]
fn the_store_is_scoped_to_its_project() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    let rel = issue.dossier_path.clone().unwrap();
    let conversation = fx.conversation(&task_id);
    telemetry::observe_acp_session(
        &fx.app,
        &conversation,
        &models(vec![tool(ToolItemKind::ReadToolCall, 0, &rel)], Some(1_000)),
    );
    assert_eq!(fx.report().sessions_observed, 1);

    let other = telemetry::memory_value(&fx.app, "prj_does_not_exist", DEFAULT_WINDOW_DAYS);
    assert_eq!(other.sessions_observed, 0);
    assert_eq!(other.citations.rate(), None);
}

// -- the PTY path ------------------------------------------------------------

/// A PTY agent session leaves fartCode a 64 KiB scrollback ring and nothing
/// else — no roles, no tool calls, no usage metadata. The two tests below
/// are the only place the managed-state lookup, the base64 tail decode, the
/// ANSI strip and the truncated-fidelity rule are exercised together.
struct Pty {
    app: tauri::App<tauri::test::MockRuntime>,
    terminal_id: String,
}

fn pty_session(task_id: &str, echo: &str) -> Pty {
    use fartcode_app_lib::terminals::{TerminalManager, TerminalSpec};
    use tauri::Manager as _;

    let app = tauri::test::mock_app();
    let manager = Arc::new(TerminalManager::new(app.handle().clone()));
    let cwd = std::env::temp_dir();
    let terminal_id = manager
        .open(TerminalSpec {
            task_id,
            project_id: "p1",
            agent: Some("claude"),
            tmux: false,
            program: "/bin/sh",
            args: &["-c".into(), format!("printf '%s\\n' '{echo}'; sleep 30")],
            env: &[],
            remove: &[],
            cwd: &cwd,
            rows: 24,
            cols: 80,
            lifecycle: None::<fartcode_core::terminals::lifecycle::LifecycleScriptType>,
        })
        .expect("open agent terminal");

    // Bounded wait for the pump to fill the tail — never an unbounded loop.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && manager.tail(&terminal_id).is_none() {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        manager.tail(&terminal_id).is_some(),
        "terminal produced no output within the deadline"
    );
    app.manage(manager);
    Pty { app, terminal_id }
}

#[test]
fn a_pty_session_that_mentions_the_dossier_is_a_weak_citation() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("OAuth login");
    let rel = issue.dossier_path.clone().unwrap();
    let pty = pty_session(&task_id, &format!("read {rel}"));

    telemetry::observe_pty_session(pty.app.handle(), &fx.app, &task_id, &pty.terminal_id);

    let report = fx.report();
    assert_eq!(report.sessions_observed, 1);
    // A scrollback ring has no tool-call structure, so the strongest a PTY
    // session can score is a mention.
    assert_eq!(report.citations.cited_mention, 1);
    assert_eq!(report.citations.cited_read, 0);
}

/// The honest half: silence from a 64 KiB window over an hour-long session
/// is not a "no", and must never be counted as a miss.
#[test]
fn a_pty_session_with_no_mention_is_unknown_not_a_miss() {
    let fx = fixture();
    let (_issue, task_id) = fx.card_mid_step("OAuth login");
    let pty = pty_session(&task_id, "nothing to see here");

    telemetry::observe_pty_session(pty.app.handle(), &fx.app, &task_id, &pty.terminal_id);

    let report = fx.report();
    assert_eq!(report.citations.unknown, 1);
    assert_eq!(report.citations.not_cited, 0);
    assert_eq!(report.citations.rate(), None, "no conclusive session");
    // With no usage metadata on this path it takes no side in the token
    // contrast either.
    assert!(matches!(
        report.tokens_saved,
        fartcode_telemetry::TokensSaved::Insufficient {
            citing_with_usage: 0,
            not_citing_with_usage: 0,
            ..
        }
    ));
}
