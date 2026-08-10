//! Local memory-value telemetry — the wired half (E19-04, #73; ADR-0038
//! item 7).
//!
//! `fartcode_telemetry` owns what the four signals *mean* and computes them
//! from values. This module is the half that needs the App: which
//! transcript to read, when to read it, where the small verdict goes, and
//! which dossiers to walk when the dashboard asks.
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
//! # Where the capture happens, and why not at the hooks
//!
//! [`observe_settled_step`] is called from `step_engine::settle_issues_observed`,
//! after the engine's `begin_settle` has returned `Act`. The two hooks that
//! *hold* a transcript — a finished ACP turn, an agent terminal exiting —
//! fire far more often than a step settles, and the engine refuses most of
//! them (parked, stale, tombstoned, repark). Observing at the hook counted
//! those refusals as steps. Threading the transcript down to the engine's
//! verdict makes one settled step run exactly one row.
//!
//! # Provenance is the whole safety property
//!
//! Both citation corrections and the re-ask tally key off
//! [`SegmentSource`]: the injected prompt names the dossier and demonstrates
//! both clarification tags, so any text containing it must not be scanned.
//! The ACP adapter has real structure to map — roles, tool kinds — and
//! preserves it. A PTY session has none: fartCode sees an agent CLI through
//! a 64 KiB scrollback ring that *echoes the bracket-pasted prompt*, so it
//! is handed over as [`SegmentSource::Unstructured`] and is not scanned at
//! all. That is argued where the variant is defined; the short version is
//! that a reconstructed prompt mask matches only partially against a TUI's
//! reflowed redraw, and a partial match quietly restores a citation rate of
//! 100%.
//!
//! # Same posture as the dossier writer
//!
//! Every entry point here returns `()` and logs. Telemetry must not fail a
//! settle, slow one, or change what the pipeline does. The capture sites run
//! off the main thread (the ACP callback and the PTY pump thread), and the
//! dashboard read is behind an `async` command that hands its body to
//! `spawn_blocking`.
//!
//! # Nothing leaves the machine
//!
//! There is no upload path here and none in `fartcode-telemetry`, whose
//! `tests/no_egress.rs` fails the build if one appears. The store is a
//! versioned-JSON value in the app's own SQLite `kv` table, written through
//! `Db::kv_update` so two concurrent settles cannot lose each other's rows.

use std::path::Path;
use std::sync::Arc;

use fartcode_core::db::versioned_json::Versioned;
use fartcode_core::dossiers;
use fartcode_core::issues::columns::BoardColumn;
use fartcode_core::issues::Issue;
use fartcode_telemetry::citations::{classify_shell, CitationTargets};
use fartcode_telemetry::memory::{self, DatedTally, MemoryInputs, MemoryValue};
use fartcode_telemetry::observation::{
    Fidelity, ObservationLog, Segment, SegmentSource, StepObservation, TranscriptView,
};
use fartcode_telemetry::reask;
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

fn parse_log(raw: Option<&str>) -> ObservationLog {
    fartcode_core::db::versioned_json::parse_versioned::<StoredLog>("telemetry_memory", raw)
        .map(|stored| stored.0)
        .unwrap_or_default()
}

/// Reads a project's log. A missing, corrupt, or future-versioned value
/// reads as empty — `parse_versioned` never throws, and losing telemetry
/// history is not a reason to fail anything.
fn load(app: &App, project_id: &str) -> ObservationLog {
    match app.db.kv_get(&key(project_id)) {
        Ok(raw) => parse_log(raw.as_deref()),
        Err(e) => {
            tracing::warn!(project_id, error = %e, "telemetry log read failed");
            ObservationLog::default()
        }
    }
}

/// Appends one observation **atomically**.
///
/// `kv_update` runs the read-modify-write under one connection guard. The
/// obvious `kv_get` / edit / `kv_set` spelling cannot: the guard may not be
/// held across those calls, so two settles landing together each read the
/// same log and the second write erases the first's row. Two agent steps
/// finishing at once is not an exotic case on a board that runs them in
/// parallel.
fn append(app: &App, project_id: &str, observation: StepObservation) {
    let mut observation = Some(observation);
    let result = app.db.kv_update(&key(project_id), &mut |current| {
        let mut log = parse_log(current.as_deref());
        if let Some(observation) = observation.take() {
            log.push(observation);
        }
        fartcode_core::db::versioned_json::serialize_versioned(&StoredLog(log)).map(Some)
    });
    if let Err(e) = result {
        tracing::warn!(project_id, error = %e, "telemetry log write failed");
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Records one settled step run against the card it settled.
///
/// Called by `step_engine::settle_issues_observed` after the engine decided
/// this step really settled. A card with no dossier records nothing: it had
/// no memory to cite, and the citation rate is a statement about steps that
/// did.
pub fn observe_settled_step(
    app: &App,
    issue: &Issue,
    column: &BoardColumn,
    session: Option<&str>,
    transcript: Option<&TranscriptView<'_>>,
) {
    let Some(rel) = issue
        .dossier_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return;
    };

    let absent = TranscriptView::absent();
    let view = transcript.unwrap_or(&absent);

    let paths = [rel.to_string()];
    let targets = CitationTargets {
        paths: &paths,
        dir: dossiers::DOSSIER_DIR,
    };
    // The FOLDED fidelity, not the one the view started with: a scan that
    // ran out of budget is no longer a full reading, and a session that can
    // only ever contribute a positive must not sit in the rate.
    let (citation, scan, fidelity) = fartcode_telemetry::citations::verdict(view, &targets);

    append(
        app,
        &issue.project_id,
        StepObservation {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            session: session.unwrap_or_default().to_string(),
            column: column.name.clone(),
            settled_at: now_secs(),
            citation,
            wrote_dossier: scan.write_hits > 0,
            reask: reask::tally(view),
            context_used: view.context_used,
            context_size: view.context_size,
            fidelity,
        },
    );
}

// ---------------------------------------------------------------------------
// Transcript adapters
// ---------------------------------------------------------------------------

/// Flattens a reduced ACP transcript into scannable, provenanced segments.
///
/// **`Role::User` maps to `InjectedPrompt`.** The seeded step prompt arrives
/// as the synthetic user message (ADR-0029), so it has to be excluded or the
/// citation rate is 100% by construction. The cost, stated: a human who
/// types "check docs/features/oauth-login.md" is also skipped, so that
/// session may read as not-cited. Conservative in the direction that
/// matters — a metric that under-reports is recoverable, one that flatters
/// itself is not.
pub fn acp_view(models: &fartcode_acp::LiveModels) -> TranscriptView<'_> {
    use fartcode_acp::transcript::{ToolItemKind, TranscriptItem};

    let mut segments: Vec<Segment<'_>> = Vec::new();

    fn push_tool<'a>(out: &mut Vec<Segment<'a>>, tool: &'a fartcode_acp::transcript::ToolCallItem) {
        // A shell call's intent is read off the command by
        // `classify_shell`, so `git add docs/features/x.md` — the staging
        // the seeded prompt's own instruction leads to — is authorship,
        // while `rg docs/features/`, the read the seeded skill teaches, is
        // a real citation. Scored flat, the first was a citation and the
        // second was nothing.
        let source = match tool.kind {
            ToolItemKind::ReadToolCall | ToolItemKind::SearchToolCall => SegmentSource::ToolRead,
            ToolItemKind::CreateFileToolCall
            | ToolItemKind::ModifyFileToolCall
            | ToolItemKind::DeleteFileToolCall => SegmentSource::ToolWrite,
            ToolItemKind::ExecuteToolCall => tool
                .command
                .as_deref()
                .map(|command| classify_shell(command).source())
                .unwrap_or(SegmentSource::ToolExec),
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

/// The agent terminal's rolling scrollback, decoded and de-ANSI'd, or
/// `None` when there is no such terminal.
///
/// Returned as owned text so the caller can borrow a [`TranscriptView`]
/// from it across the settle.
pub fn pty_scrollback<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    terminal_id: &str,
) -> Option<String> {
    use tauri::Manager as _;

    let tail = handle
        .try_state::<Arc<crate::terminals::TerminalManager<R>>>()?
        .tail(terminal_id)?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(tail)
        .ok()?;
    Some(strip_ansi(&String::from_utf8_lossy(&bytes)))
}

/// A view over PTY scrollback.
///
/// **Everything here is unscannable and truncated, on purpose.** See
/// [`SegmentSource::Unstructured`]: the tail echoes the bracket-pasted step
/// prompt, which contains the dossier path and demonstrates both
/// clarification tags, and there is no provenance left to exclude it with.
/// So a PTY step yields `Citation::Unknown` and an unreadable re-ask
/// observation rather than a hit it did not earn. Its tags are still
/// counted — from the section it committed to the dossier, which carries no
/// echo.
pub fn pty_view(scrollback: Option<&str>) -> TranscriptView<'_> {
    match scrollback {
        Some(text) => TranscriptView {
            segments: vec![Segment::new(SegmentSource::Unstructured, text)],
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Truncated,
        },
        None => TranscriptView::absent(),
    }
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

/// Builds the report. Blocking (SQLite + a bounded set of small file reads)
/// — callers must already be off the main thread.
///
/// The window applies to **every** signal, not just the observations: cycles
/// are filtered by landing time and dossier sections by the date in their
/// heading, both inside `memory::compute`. A section whose heading carries
/// no date is excluded and counted, because a report labelled "the last 90
/// days" that silently spans all history is a wrong number, not a rough one.
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
    let mut dossier_tallies: Vec<DatedTally> = Vec::new();
    let mut dossiers_scanned = 0u32;

    // Derived from the heading the appender writes, not spelled out again:
    // one string, so a rename cannot leave this reading a section that no
    // longer exists.
    let timeline = dossiers::TIMELINE_HEADING.trim_start_matches("## ");

    for issue in &issues {
        let Some(content) = read_dossier(app, issue) else {
            continue;
        };
        dossiers_scanned += 1;
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
                    dossier_tallies.push(DatedTally {
                        at: time_to_land::section_date(&section.heading),
                        tally,
                    });
                }
            }
        }
    }

    memory::compute(MemoryInputs {
        project_id,
        window_days,
        since,
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
    use fartcode_telemetry::observation::ReAskObservation;
    use fartcode_telemetry::reask::{TAG_HUMAN, TAG_MEMORY};

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

    /// The headline of the fix round, at the unit level: a terminal echoes
    /// the seeded prompt, which names the dossier and shows both tags. The
    /// PTY view must yield nothing from either.
    #[test]
    fn a_pty_view_of_an_echoed_prompt_scores_nothing() {
        let echoed = format!(
            "$ claude\r\n\u{1b}[2m> This feature keeps a decision log at \
             `docs/features/oauth-login.md`.\u{1b}[0m\r\n\
             > {TAG_MEMORY} <question> — <where>\r\n\
             > {TAG_HUMAN} <question>\r\n\
             thinking...\r\n"
        );
        let stripped = strip_ansi(&echoed);
        let view = pty_view(Some(&stripped));
        let paths = ["docs/features/oauth-login.md".to_string()];
        let targets = CitationTargets {
            paths: &paths,
            dir: dossiers::DOSSIER_DIR,
        };
        let (citation, scan, fidelity) = fartcode_telemetry::citations::verdict(&view, &targets);
        assert_eq!(citation, fartcode_telemetry::Citation::Unknown);
        assert_eq!(scan, fartcode_telemetry::CitationScan::default());
        assert_eq!(fidelity, Fidelity::Truncated);
        assert_eq!(reask::tally(&view), ReAskObservation::Unreadable);
    }

    #[test]
    fn no_scrollback_is_an_absent_view() {
        let view = pty_view(None);
        assert!(view.segments.is_empty());
        assert_eq!(view.fidelity, Fidelity::Absent);
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
        log.push(StepObservation {
            project_id: "prj_1".into(),
            issue_id: "iss_1".into(),
            session: "acp:c1".into(),
            column: "Plan".into(),
            settled_at: 42,
            citation: fartcode_telemetry::Citation::CitedRead,
            wrote_dossier: true,
            reask: ReAskObservation::Scanned(fartcode_telemetry::ReAskTally {
                memory_answered: 2,
                human_asked: 1,
            }),
            context_used: Some(1_000),
            context_size: Some(200_000),
            fidelity: Fidelity::Full,
        });
        let json = fartcode_core::db::versioned_json::serialize_versioned(&StoredLog(log.clone()))
            .unwrap();
        assert!(json.contains("\"version\":1"), "{json}");
        assert_eq!(parse_log(Some(&json)), log);

        // Never throws: junk and absence both read as an empty log.
        assert_eq!(parse_log(Some("{not json")), ObservationLog::default());
        assert_eq!(parse_log(None), ObservationLog::default());
    }
}
