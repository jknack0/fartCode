//! Local memory-value telemetry — the wired half (E19-04, #73; ADR-0038
//! item 7).
//!
//! `fartcode_telemetry` owns what the four signals *mean* and computes them
//! from values. This module is the half that needs the App: which transcript
//! to read, when to read it, where the small verdict goes, and which
//! dossiers to walk when the dashboard asks.
//!
//! # Capture at settle, compute on demand
//!
//! Three of the four signals need a transcript, and fartCode persists none:
//! the ACP reducer's state and its raw log live in the `SessionCell` and die
//! with it (ADR-0029 item 5), and the `messages` table has had no reader or
//! writer since Phase 0. So there is no "read it later" — the scan happens
//! at settle, while the reduced transcript is still in memory, and what is
//! kept is a `StepObservation`: a fixed-size verdict with **no transcript
//! text in it at all**.
//!
//! The fourth signal, time-to-land, needs nothing captured. `## Timeline`
//! breadcrumbs are committed to the branch, so it is read from the
//! repository when the dashboard asks and never touches a hot path.
//!
//! # Same posture as the dossier writer
//!
//! Every entry point here returns `()` and logs. Telemetry must not fail a
//! settle, slow one, or change what the pipeline does — a memory-value
//! number is worth strictly less than the step it would have broken. All
//! four capture sites already run off the main thread (the ACP callback and
//! the PTY pump thread), and the reads the dashboard needs are behind an
//! `async` command that hands its body to `spawn_blocking`.
//!
//! # Nothing leaves the machine
//!
//! There is no upload path here and none in `fartcode-telemetry`, whose
//! `tests/no_egress.rs` fails the build if one appears. The store is a
//! versioned-JSON value in the app's own SQLite `kv` table.

use std::path::Path;
use std::sync::Arc;

use fartcode_core::conversations::ConversationStore as _;
use fartcode_core::db::versioned_json::Versioned;
use fartcode_core::dossiers;
use fartcode_core::issues::Issue;
use fartcode_telemetry::citations::CitationTargets;
use fartcode_telemetry::memory::{self, MemoryInputs, MemoryValue};
use fartcode_telemetry::observation::{
    Fidelity, ObservationLog, Segment, SegmentSource, StepObservation, TranscriptView,
};
use fartcode_telemetry::reask::{self, ReAskTally};
use fartcode_telemetry::time_to_land::{self, FeatureCycle};

use crate::app::App;

/// kv key prefix. One value per project, so a project's history is one row
/// to read and one to rewrite.
const KV_PREFIX: &str = "telemetry:memory:";

/// Versioned wrapper for the stored log.
///
/// A newtype because `Versioned` belongs to `fartcode-core` and
/// `ObservationLog` to `fartcode-telemetry` — both foreign to this crate,
/// so the orphan rule needs a local type. `transparent` keeps the stored
/// JSON identical to the log's own.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct StoredLog(ObservationLog);

impl Versioned for StoredLog {
    const VERSION: u32 = 1;
}

fn key(project_id: &str) -> String {
    format!("{KV_PREFIX}{project_id}")
}

/// Reads a project's log. A missing, corrupt, or future-versioned value
/// reads as empty — `parse_versioned` never throws, and losing telemetry
/// history is not a reason to fail anything.
fn load(app: &App, project_id: &str) -> ObservationLog {
    let raw = match app.db.kv_get(&key(project_id)) {
        Ok(raw) => raw,
        Err(e) => {
            tracing::warn!(project_id, error = %e, "telemetry log read failed");
            return ObservationLog::default();
        }
    };
    fartcode_core::db::versioned_json::parse_versioned::<StoredLog>(
        "telemetry_memory",
        raw.as_deref(),
    )
    .map(|stored| stored.0)
    .unwrap_or_default()
}

fn save(app: &App, project_id: &str, log: &ObservationLog) {
    // `ObservationLog` is `Clone` and small; the serializer needs an owned
    // newtype and this is the whole cost of it.
    let stored = StoredLog(log.clone());
    match fartcode_core::db::versioned_json::serialize_versioned(&stored) {
        Ok(json) => {
            if let Err(e) = app.db.kv_set(&key(project_id), &json) {
                tracing::warn!(project_id, error = %e, "telemetry log write failed");
            }
        }
        Err(e) => tracing::warn!(project_id, error = %e, "telemetry log serialize failed"),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Capture — ACP
// ---------------------------------------------------------------------------

/// Records one settled ACP session against every dossier-bearing card its
/// task owns.
///
/// Called from `acp_events::transcript_changed` on the newly-done-turn
/// edge — the same edge that already triggers the auto-flip — and **before**
/// the flip, because an advancing settle moves the card out of the column
/// this observation is about.
///
/// Idempotent by construction: the hook fires once per turn that settles
/// Done, and `ObservationLog::record` replaces the row for this
/// `(card, session)` instead of appending, so a ten-turn session is one
/// observation reflecting the whole transcript rather than ten nested
/// supersets of it.
pub fn observe_acp_session(app: &App, conversation_id: &str, models: &fartcode_acp::LiveModels) {
    let Ok(Some(conversation)) = app.conversations.get(conversation_id) else {
        return;
    };
    // A project-scoped conversation (the PM chat) settles nothing and has
    // no feature to cite.
    let Some(task_id) = conversation.task_id.as_deref() else {
        return;
    };
    let view = acp_view(models);
    record_for_task(app, task_id, &format!("acp:{conversation_id}"), &view);
}

/// Flattens a reduced transcript into scannable segments.
///
/// **`Role::User` maps to `InjectedPrompt`.** The seeded step prompt arrives
/// as the synthetic user message (ADR-0029), so it has to be excluded or the
/// citation rate is 100% by construction. The cost, stated: a human who
/// types "check docs/features/oauth-login.md" is also skipped, so that
/// session may read as not-cited. Conservative in the direction that
/// matters — a metric that under-reports is recoverable, one that flatters
/// itself is not.
fn acp_view(models: &fartcode_acp::LiveModels) -> TranscriptView<'_> {
    use fartcode_acp::transcript::{ToolItemKind, TranscriptItem};

    let mut segments: Vec<Segment<'_>> = Vec::new();

    fn push_tool<'a>(out: &mut Vec<Segment<'a>>, tool: &'a fartcode_acp::transcript::ToolCallItem) {
        let source = match tool.kind {
            ToolItemKind::ReadToolCall | ToolItemKind::SearchToolCall => SegmentSource::ToolRead,
            ToolItemKind::CreateFileToolCall
            | ToolItemKind::ModifyFileToolCall
            | ToolItemKind::DeleteFileToolCall => SegmentSource::ToolWrite,
            ToolItemKind::ExecuteToolCall => SegmentSource::ToolExec,
            _ => SegmentSource::Other,
        };
        // Fields that can name a path. `content` / `new_text` / `old_text`
        // are deliberately NOT scanned: they are file bodies, and a dossier
        // that quotes its own path would then cite itself.
        for text in [
            Some(tool.title.as_str()),
            tool.input_summary.as_deref(),
            tool.path.as_deref(),
            tool.query.as_deref(),
            tool.command.as_deref(),
            tool.output_text.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|t| !t.is_empty())
        {
            out.push(Segment::new(source, text));
        }
        for child in tool.children.iter().flatten() {
            push_node(out, child);
        }
    }

    fn push_node<'a>(out: &mut Vec<Segment<'a>>, node: &'a fartcode_acp::transcript::ToolNode) {
        match node {
            fartcode_acp::transcript::ToolNode::Tool(tool) => push_tool(out, tool),
            fartcode_acp::transcript::ToolNode::Group(group) => {
                for child in &group.children {
                    push_node(out, child);
                }
            }
        }
    }

    for turn in models.committed.iter().chain(models.active_turn.iter()) {
        for item in &turn.items {
            match item {
                TranscriptItem::Message(message) => {
                    let source = match message.role {
                        fartcode_acp::transcript::Role::User => SegmentSource::InjectedPrompt,
                        fartcode_acp::transcript::Role::Assistant => SegmentSource::AgentProse,
                    };
                    segments.push(Segment::new(source, &message.text));
                }
                TranscriptItem::Thinking(thinking) => {
                    segments.push(Segment::new(SegmentSource::AgentProse, &thinking.text));
                }
                TranscriptItem::Tool(tool) => push_tool(&mut segments, tool),
                TranscriptItem::Group(group) => {
                    for child in &group.children {
                        push_node(&mut segments, child);
                    }
                }
            }
        }
    }

    TranscriptView {
        segments,
        context_used: models.usage.as_ref().map(|u| u.context_used),
        context_size: models.usage.as_ref().map(|u| u.context_size),
        fidelity: Fidelity::Full,
    }
}

// ---------------------------------------------------------------------------
// Capture — PTY
// ---------------------------------------------------------------------------

/// Records one settled agent-terminal session from the terminal's rolling
/// scrollback.
///
/// **Everything a PTY session yields is [`Fidelity::Truncated`].** fartCode
/// runs the agent CLI in a terminal and keeps a 64 KiB tail for reattach
/// replay; there is no transcript, no roles, no tool-call structure, and the
/// beginning of a long session has already been evicted. So a hit still
/// counts (a mention is a mention) but a *miss* resolves to
/// `Citation::Unknown`, never to "did not cite" — and no usage metadata
/// exists on this path at all, so PTY sessions never enter the token
/// contrast.
pub fn observe_pty_session<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    app: &App,
    task_id: &str,
    terminal_id: &str,
) {
    use tauri::Manager as _;

    let session = format!("pty:{terminal_id}");
    let text = handle
        .try_state::<Arc<crate::terminals::TerminalManager<R>>>()
        .and_then(|terminals| terminals.tail(terminal_id))
        .and_then(|b64| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
        })
        .map(|bytes| strip_ansi(&String::from_utf8_lossy(&bytes)));

    let view = match text.as_deref() {
        Some(text) => TranscriptView {
            segments: vec![Segment::new(SegmentSource::Other, text)],
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Truncated,
        },
        None => TranscriptView::absent(),
    };
    record_for_task(app, task_id, &session, &view);
}

/// Removes CSI/OSC escape sequences so a path split by a colour change is
/// still one string. Deliberately crude — it is a scan input, not something
/// anybody reads.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters then one final byte in @..~
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: runs to BEL or ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other two-character escape: both consumed.
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shared recording path
// ---------------------------------------------------------------------------

fn record_for_task(app: &App, task_id: &str, session: &str, view: &TranscriptView<'_>) {
    let issues = match app.issues.list_by_linked_task(task_id) {
        Ok(issues) => issues,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "telemetry: issue lookup failed");
            return;
        }
    };
    let settled_at = now_secs();
    let reask = reask::tally(view);

    for issue in issues {
        // No dossier means nothing to cite — and the card never consented
        // to one either. Skipping keeps the citation rate a statement about
        // steps that HAD memory available, which is the only version of the
        // number that answers "is this feature worth it".
        let Some(rel) = issue
            .dossier_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let paths = [rel.to_string()];
        let targets = CitationTargets {
            paths: &paths,
            dir: dossiers::DOSSIER_DIR,
        };
        let (citation, _) = fartcode_telemetry::citations::verdict(view, &targets);

        let observation = StepObservation {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            session: session.to_string(),
            column: column_name(app, &issue),
            settled_at,
            citation,
            reask,
            context_used: view.context_used,
            context_size: view.context_size,
            fidelity: view.fidelity,
        };

        let mut log = load(app, &issue.project_id);
        log.record(observation);
        save(app, &issue.project_id, &log);
    }
}

fn column_name(app: &App, issue: &Issue) -> String {
    issue
        .column_id
        .as_deref()
        .and_then(|id| app.columns.get(id).ok().flatten())
        .map(|column| column.name)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// On-demand aggregation (the dashboard, #76)
// ---------------------------------------------------------------------------

/// How many dossiers one report will read.
///
/// **The row cap on the repository side.** Time-to-land and the durable
/// half of the re-ask signal both walk `docs/features/`, one small file per
/// card, on the thread the command offloaded to. 200 features is more than
/// any project that has adopted dossiers has today and keeps the walk at a
/// few hundred small reads; past it the report says `clipped` rather than
/// growing without bound. Cards are visited newest-first so the cap trims
/// the least relevant end.
const MAX_DOSSIERS: usize = 200;

/// Largest dossier this will read. A dossier is prose; anything past a
/// megabyte is not one, and `parse_timeline` should not be handed a blob
/// somebody committed by accident.
const MAX_DOSSIER_BYTES: u64 = 1024 * 1024;

/// Builds the report. Blocking (SQLite + a bounded directory of small file
/// reads) — callers must already be off the main thread.
pub fn memory_value(app: &App, project_id: &str, window_days: u32) -> MemoryValue {
    let since = now_secs() - i64::from(window_days) * 86_400;
    let log = load(app, project_id);
    let observations = log.window(project_id, since);

    let mut issues = match app.issues.list_for_project(project_id) {
        Ok(issues) => issues,
        Err(e) => {
            tracing::warn!(project_id, error = %e, "telemetry: issue list failed");
            Vec::new()
        }
    };
    issues.retain(|issue| issue.dossier_path.is_some());
    // Newest first, so the cap drops the oldest features rather than an
    // arbitrary slice.
    issues.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let clipped_dossiers = issues.len() > MAX_DOSSIERS;
    issues.truncate(MAX_DOSSIERS);

    let mut cycles: Vec<FeatureCycle> = Vec::new();
    let mut dossier_tallies: Vec<ReAskTally> = Vec::new();
    let mut dossiers_scanned = 0u32;

    for issue in &issues {
        let Some(content) = read_dossier(app, issue) else {
            continue;
        };
        dossiers_scanned += 1;
        // Derived from the heading the appender writes, not spelled out
        // again: one string, so a rename cannot leave this reading a
        // section that no longer exists.
        let timeline = dossiers::TIMELINE_HEADING.trim_start_matches("## ");
        for section in dossiers::sections(&content) {
            if section.heading == timeline {
                let events = time_to_land::parse_timeline(&section.body);
                if let Some(cycle) = time_to_land::cycle_of(&events) {
                    cycles.push(cycle);
                }
            } else if !dossiers::is_app_section(&section.heading) {
                // Agent-written sections only: the app-written header is
                // copied from card text, and a card body that happened to
                // contain the tag literal must not be counted as an agent's
                // clarification.
                let tally = reask::tally_text(&section.body);
                if !tally.is_silent() {
                    dossier_tallies.push(tally);
                }
            }
        }
    }

    memory::compute(MemoryInputs {
        project_id,
        window_days,
        observations: &observations,
        dossier_tallies: &dossier_tallies,
        cycles,
        dossiers_scanned,
        clipped: clipped_dossiers || log.is_full(),
    })
}

/// The freshest copy of a card's dossier that is provably ITS dossier —
/// `dossier_index`'s resolver, reused rather than re-derived, so telemetry
/// and ⌘K can never disagree about which file a card owns.
fn read_dossier(app: &App, issue: &Issue) -> Option<String> {
    let rel = issue.dossier_path.as_deref()?.trim();
    if rel.is_empty() {
        return None;
    }
    let path = crate::dossier_index::dossier_source(app, issue, rel)?;
    if too_big(&path) {
        tracing::warn!(path = %path.display(), "telemetry: dossier too large, skipped");
        return None;
    }
    std::fs::read_to_string(&path).ok()
}

fn too_big(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_DOSSIER_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_stripping_rejoins_a_colourised_path() {
        let raw = "\u{1b}[32mdocs/features/\u{1b}[0moauth-login.md\u{1b}[0m";
        assert_eq!(strip_ansi(raw), "docs/features/oauth-login.md");
    }

    #[test]
    fn ansi_stripping_handles_osc_and_leaves_plain_text_alone() {
        assert_eq!(strip_ansi("\u{1b}]0;a title\u{7}hello"), "hello");
        assert_eq!(strip_ansi("\u{1b}]0;a title\u{1b}\\hello"), "hello");
        assert_eq!(strip_ansi("plain text"), "plain text");
        // An unterminated escape at EOF must not panic or loop.
        assert_eq!(strip_ansi("tail\u{1b}["), "tail");
    }

    #[test]
    fn the_kv_key_is_project_scoped() {
        assert_eq!(key("prj_1"), "telemetry:memory:prj_1");
        assert_ne!(key("prj_1"), key("prj_2"));
    }

    /// The stored value is versioned JSON like every other JSON the app
    /// keeps, so a format change is a version bump rather than a silent
    /// misparse.
    #[test]
    fn the_log_round_trips_through_versioned_json() {
        let mut log = ObservationLog::default();
        log.record(StepObservation {
            project_id: "prj_1".into(),
            issue_id: "iss_1".into(),
            session: "acp:c1".into(),
            column: "Plan".into(),
            settled_at: 42,
            citation: fartcode_telemetry::Citation::CitedRead,
            reask: ReAskTally {
                memory_answered: 2,
                human_asked: 1,
            },
            context_used: Some(1_000),
            context_size: Some(200_000),
            fidelity: Fidelity::Full,
        });
        let json = fartcode_core::db::versioned_json::serialize_versioned(&StoredLog(log.clone()))
            .unwrap();
        assert!(json.contains("\"version\":1"), "{json}");
        let back = fartcode_core::db::versioned_json::parse_versioned::<StoredLog>(
            "telemetry_memory",
            Some(&json),
        )
        .unwrap();
        assert_eq!(back.0, log);

        // Never throws: junk and absence both read as an empty log.
        assert!(
            fartcode_core::db::versioned_json::parse_versioned::<StoredLog>(
                "telemetry_memory",
                Some("{not json"),
            )
            .is_none()
        );
        assert!(
            fartcode_core::db::versioned_json::parse_versioned::<StoredLog>(
                "telemetry_memory",
                None,
            )
            .is_none()
        );
    }
}
