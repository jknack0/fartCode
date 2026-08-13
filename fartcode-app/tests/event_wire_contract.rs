//! Golden-file contract test for the `fartcode:event` frontend envelope.
//!
//! `event_to_value` is the only path from `InternalEvent` to the webview, and
//! the frontend types (`FartcodeEvent` in app-frontend/src/lib/tauri.ts) are
//! hand-written against its exact output — including shipped quirks like
//! task:created carrying "id" while task:deleted carries "taskId"
//! (audits/2026-08-12-architecture-deep-dive.md, T1). This test pins every
//! envelope BYTE-FOR-BYTE against tests/snapshots/events.json so the wire
//! format can only change deliberately, in a diff a reviewer sees.
//!
//! Every `InternalEvent` variant gets exactly one fixture: forwarded events
//! snapshot their envelope, backend-internal events snapshot `null`. The
//! `ordinal` match is wildcard-free, so adding a variant fails compilation
//! here until it gets a fixture — and then fails the run until the snapshot
//! records its wire shape (or its absence).
//!
//! To regenerate after an INTENDED wire change (frontend updated in the same
//! change):
//!     FARTCODE_UPDATE_SNAPSHOTS=1 cargo test -p fartcode-app --test event_wire_contract

use std::path::PathBuf;

use fartcode_app_lib::app::event_to_value;
use fartcode_core::events::InternalEvent;
use serde_json::Value;

/// One fixture per variant, in enum declaration order. Values are arbitrary
/// but FROZEN — changing one invalidates the snapshot bytes.
fn fixtures() -> Vec<InternalEvent> {
    use InternalEvent as E;
    vec![
        E::AppStarted,
        E::AppClosed { was_crash: false },
        E::ProjectAdded {
            id: "p1".into(),
            name: "demo".into(),
            path: "/repo/demo".into(),
        },
        E::ProjectDeleted { id: "p1".into() },
        E::TaskCreated {
            id: "t1".into(),
            project_id: "p1".into(),
            name: "fix".into(),
        },
        E::TaskRenamed {
            id: "t1".into(),
            project_id: "p1".into(),
            name: "renamed".into(),
        },
        E::TaskProvisioned {
            id: "t1".into(),
            workspace_id: "w1".into(),
        },
        E::TaskStatusChanged {
            id: "t1".into(),
            old_status: "open".into(),
            new_status: "running".into(),
        },
        E::TaskArchived { id: "t1".into() },
        E::TaskRestored { id: "t1".into() },
        E::TaskDeleted { id: "t1".into() },
        E::ByoiTerminateWarning {
            task_id: "t1".into(),
            message: "terminate script failed".into(),
        },
        E::CommentCreated {
            id: "lc_1".into(),
            task_id: "t1".into(),
            file_path: "src/main.rs".into(),
            line_number: 42,
        },
        E::CommentResolved { id: "lc_1".into() },
        E::IssueCreated {
            id: "i1".into(),
            project_id: "p1".into(),
            title: "card".into(),
        },
        E::IssueUpdated {
            id: "i1".into(),
            project_id: "p1".into(),
        },
        E::IssueDeleted {
            id: "i1".into(),
            project_id: "p1".into(),
        },
        E::IssueColumnChanged {
            id: "i1".into(),
            project_id: "p1".into(),
            from: Some("c1".into()),
            to: "c2".into(),
        },
        E::StepLaunch {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            task_id: "t1".into(),
            prompt: "go".into(),
            provider: "claude".into(),
            model: Some("haiku".into()),
            effort: None,
            reattached: false,
        },
        E::StepQueued {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            provider: "claude".into(),
            model: None,
            effort: Some("high".into()),
        },
        E::StepQueueCleared {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
        },
        E::StepSettled {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            task_id: "t1".into(),
        },
        E::StepChainHeld {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            target_column_id: Some("c2".into()),
            reason: "budget".into(),
        },
        E::ConversationCreated {
            id: "cv1".into(),
            task_id: "t1".into(),
            provider: "claude".into(),
            title: "chat".into(),
        },
        E::ConversationRenamed {
            id: "cv1".into(),
            title: "renamed".into(),
        },
        E::ConversationDeleted { id: "cv1".into() },
        E::AgentStart {
            provider: "claude".into(),
            project_id: "p1".into(),
            task_id: "t1".into(),
            conversation_id: "cv1".into(),
        },
        E::AgentRunStarted {
            conversation_id: "cv1".into(),
            provider: "claude".into(),
        },
        E::AgentRunFinished {
            conversation_id: "cv1".into(),
            provider: "claude".into(),
            exit_code: 0,
        },
        E::AgentSessionExited {
            conversation_id: "cv1".into(),
        },
        E::PtyOutput {
            pty_id: "pty1".into(),
            data: vec![104, 105],
        },
        E::PtyClosed {
            pty_id: "pty1".into(),
        },
        E::SshConnectionStateChanged {
            connection_id: "s1".into(),
            state: "reconnecting".into(),
            attempt: Some(3),
            delay_ms: Some(5000),
            error: None,
        },
        E::SshConnectionHealthChanged {
            connection_id: "s1".into(),
            degraded: true,
        },
        E::TerminalCreated {
            id: "term1".into(),
            task_id: "t1".into(),
        },
        E::TerminalDeleted { id: "term1".into() },
        E::LifecycleScriptStatusChanged {
            session_id: "sess1".into(),
            script_type: "setup".into(),
            status: "running".into(),
        },
        E::GitChanged {
            project_id: "p1".into(),
            workspace_id: "w1".into(),
        },
        E::PrUpdated {
            workspace_id: "w1".into(),
            pr_url: "https://github.com/o/r/pull/42".into(),
        },
        E::FilesChanged {
            workspace_id: "w1".into(),
            paths: vec!["src/main.rs".into()],
        },
        E::SettingChanged {
            key: "default_agent".into(),
        },
        E::SidebarToggled { visible: true },
        E::Error {
            context: "boot".into(),
            message: "nope".into(),
        },
    ]
}

/// Wildcard-free on purpose: a new `InternalEvent` variant must not compile
/// until it appears here, in `fixtures`, and in the snapshot.
fn ordinal(event: &InternalEvent) -> usize {
    use InternalEvent as E;
    match event {
        E::AppStarted => 0,
        E::AppClosed { .. } => 1,
        E::ProjectAdded { .. } => 2,
        E::ProjectDeleted { .. } => 3,
        E::TaskCreated { .. } => 4,
        E::TaskRenamed { .. } => 5,
        E::TaskProvisioned { .. } => 6,
        E::TaskStatusChanged { .. } => 7,
        E::TaskArchived { .. } => 8,
        E::TaskRestored { .. } => 9,
        E::TaskDeleted { .. } => 10,
        E::ByoiTerminateWarning { .. } => 11,
        E::CommentCreated { .. } => 12,
        E::CommentResolved { .. } => 13,
        E::IssueCreated { .. } => 14,
        E::IssueUpdated { .. } => 15,
        E::IssueDeleted { .. } => 16,
        E::IssueColumnChanged { .. } => 17,
        E::StepLaunch { .. } => 18,
        E::StepQueued { .. } => 19,
        E::StepQueueCleared { .. } => 20,
        E::StepSettled { .. } => 21,
        E::StepChainHeld { .. } => 22,
        E::ConversationCreated { .. } => 23,
        E::ConversationRenamed { .. } => 24,
        E::ConversationDeleted { .. } => 25,
        E::AgentStart { .. } => 26,
        E::AgentRunStarted { .. } => 27,
        E::AgentRunFinished { .. } => 28,
        E::AgentSessionExited { .. } => 29,
        E::PtyOutput { .. } => 30,
        E::PtyClosed { .. } => 31,
        E::SshConnectionStateChanged { .. } => 32,
        E::SshConnectionHealthChanged { .. } => 33,
        E::TerminalCreated { .. } => 34,
        E::TerminalDeleted { .. } => 35,
        E::LifecycleScriptStatusChanged { .. } => 36,
        E::GitChanged { .. } => 37,
        E::PrUpdated { .. } => 38,
        E::FilesChanged { .. } => 39,
        E::SettingChanged { .. } => 40,
        E::SidebarToggled { .. } => 41,
        E::Error { .. } => 42,
    }
}

const INTERNAL_EVENT_VARIANTS: usize = 43;

/// Snapshot key: the enum's serde tag (snake_case variant name) — stable
/// across wire renames, so a renamed frontend event still diffs in place.
fn tag(event: &InternalEvent) -> String {
    serde_json::to_value(event).expect("InternalEvent serializes")["type"]
        .as_str()
        .expect("serde tag is a string")
        .to_string()
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/events.json")
}

#[test]
fn frontend_event_envelopes_match_snapshot() {
    let fixtures = fixtures();

    // Exactly one fixture per variant (the count constant fails the build's
    // author, the seen-array fails a duplicate hiding a gap).
    assert_eq!(fixtures.len(), INTERNAL_EVENT_VARIANTS);
    let mut seen = [false; INTERNAL_EVENT_VARIANTS];
    for event in &fixtures {
        let idx = ordinal(event);
        assert!(!seen[idx], "duplicate fixture for `{}`", tag(event));
        seen[idx] = true;
    }

    // The produced wire map: serde tag -> envelope (null = not forwarded).
    let mut produced = serde_json::Map::new();
    for event in &fixtures {
        produced.insert(tag(event), event_to_value(event).unwrap_or(Value::Null));
    }
    let produced = Value::Object(produced);

    let path = snapshot_path();
    if std::env::var_os("FARTCODE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshot dir")).expect("mkdir snapshots");
        let pretty = serde_json::to_string_pretty(&produced).expect("snapshot serializes");
        std::fs::write(&path, format!("{pretty}\n")).expect("write snapshot");
    }

    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing snapshot {}: {e}\nrun once with FARTCODE_UPDATE_SNAPSHOTS=1 to create it",
            path.display()
        )
    });
    let snapshot: Value = serde_json::from_str(&raw).expect("snapshot parses");
    let snapshot = snapshot.as_object().expect("snapshot is an object");
    let produced = produced.as_object().expect("produced is an object");

    for (name, wire) in produced {
        let expected = snapshot.get(name).unwrap_or_else(|| {
            panic!(
                "no snapshot entry for `{name}` — new or renamed event; add it deliberately \
                 (FARTCODE_UPDATE_SNAPSHOTS=1) together with the frontend listener types"
            )
        });
        // Byte comparison of the serialized forms — key ORDER and key SET
        // both gate, not just structural equality.
        assert_eq!(
            serde_json::to_string(wire).expect("wire serializes"),
            serde_json::to_string(expected).expect("expected serializes"),
            "wire drift for `{name}`: the frontend types this exact shape \
             (app-frontend/src/lib/tauri.ts); change both sides and regenerate \
             with FARTCODE_UPDATE_SNAPSHOTS=1"
        );
    }
    for name in snapshot.keys() {
        assert!(
            produced.contains_key(name),
            "stale snapshot entry `{name}` has no fixture — removed event? regenerate deliberately"
        );
    }
}
