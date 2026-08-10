//! What the app observes about one settled step session, and the bounded
//! log it goes into.
//!
//! **Why anything is stored at all.** Nothing in fartCode persists a
//! transcript: the ACP reducer's state and its 50k-entry raw log live in
//! the `SessionCell` and die with it (ADR-0029 item 5), and the `messages`
//! table has had no reader or writer since Phase 0. A dashboard opened
//! tomorrow cannot go looking for yesterday's transcript, because there
//! isn't one. So the scan happens at settle, while the reduced transcript
//! is still in hand, and what survives is the **verdict, never the text**:
//! one [`StepObservation`] of a few dozen bytes per settled step.
//!
//! That is also the privacy posture. No prompt, no agent output, no file
//! content, and no path ever enters the store — a leaked observation log
//! says "some step in project P cited its dossier", nothing more.

use serde::{Deserialize, Serialize};

use crate::citations::Citation;
use crate::reask::ReAskTally;

/// How completely the session's transcript could be read at settle.
///
/// The distinction is load-bearing, not decorative: a *negative* citation
/// result over a [`Fidelity::Truncated`] transcript is not evidence of
/// absence, so it must degrade to [`Citation::Unknown`] rather than count
/// as a miss. Positives are sound at every fidelity — a mention is a
/// mention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fidelity {
    /// The whole session was available: a reduced ACP transcript, with its
    /// turns, tool calls and roles intact.
    Full,
    /// Only a bounded tail was available. The PTY path has no transcript —
    /// fartCode sees an agent CLI through a 64 KiB scrollback ring, with
    /// the beginning of a long session already evicted.
    Truncated,
    /// Nothing was available — the session was gone, or was never one this
    /// process could see.
    Absent,
}

/// Where a piece of transcript text came from. The provenance decides how
/// much a dossier mention inside it is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentSource {
    /// Text fartCode itself injected — the seeded step prompt.
    ///
    /// **Never scanned.** `skills::append_instruction` embeds the dossier
    /// path in every seeded prompt, so counting this segment would score a
    /// citation for every consented step before the agent did anything at
    /// all. That single mistake would make the headline number 100% by
    /// construction, which is precisely the self-flattering metric
    /// ADR-0038 item 7 exists to avoid.
    InjectedPrompt,
    /// Agent prose or thinking — the agent talked about the dossier.
    AgentProse,
    /// A read-ish tool call (read file, grep/search). The strongest
    /// evidence available that memory was **consumed** rather than merely
    /// named.
    ToolRead,
    /// A write-ish tool call (create/modify/delete file). Evidence the
    /// dossier was **authored**, which is the opposite of a citation — the
    /// seeded prompt asks every step to append a section, so a write
    /// mention is the expected background, not a signal.
    ToolWrite,
    /// A shell command whose intent could not be read either way — see
    /// [`crate::citations::classify_shell`], which routes the ones that
    /// can be (`rg docs/features/` reads, `git add docs/features/x.md`
    /// writes) to [`SegmentSource::ToolRead`] and
    /// [`SegmentSource::ToolWrite`] instead. What is left over is scored at
    /// the weaker prose tier.
    ToolExec,
    /// Text with no provenance at all: a flattened terminal scrollback.
    ///
    /// **Never scanned, and this is the more important of the two
    /// exclusions.** A PTY session is an agent CLI in a terminal, and the
    /// terminal echoes the bracket-pasted step prompt — which contains the
    /// dossier path *and* both clarification tags, because the prompt is
    /// what teaches them. Flattening that into scannable text bypasses
    /// [`SegmentSource::InjectedPrompt`] entirely and hands back a citation
    /// rate of 100% and a fabricated re-ask rate, on what is currently the
    /// mainstream dispatch path.
    ///
    /// **Why not mask the echo instead.** The seeded prompt is
    /// reconstructible at settle (`skills::append_instruction` is a pure
    /// function of column and path), so masking its span looks available.
    /// It is not sound: the echo is re-rendered by the agent's own TUI,
    /// reflowed at terminal width, and often redrawn or truncated, so a
    /// reconstructed span matches partially at best — and a partial match
    /// leaves the path visible and quietly restores the 100% behaviour.
    /// A heuristic whose failure mode is "the metric flatters itself again"
    /// is worse than no reading at all, so a flattened scrollback yields
    /// [`crate::Citation::Unknown`] and a silent tally. Under-reporting is
    /// recoverable; 100%-by-construction is not.
    ///
    /// A PTY agent that really does tag its clarifications is still counted
    /// — through the section it committed to the dossier, which is read
    /// separately and carries no echo.
    Unstructured,
    /// Anything else with structure but no useful classification — an
    /// unrecognized tool kind.
    Other,
}

impl SegmentSource {
    /// Whether this segment is scanned at all. Two sources are excluded,
    /// for the same underlying reason: the injected prompt names the
    /// dossier and demonstrates the tags, so any text that contains it — in
    /// its own right, or echoed into an unprovenanced blob — would score
    /// against the very thing it asked for.
    pub fn scannable(&self) -> bool {
        !matches!(
            self,
            SegmentSource::InjectedPrompt | SegmentSource::Unstructured
        )
    }

    /// Whether a hit here counts as "the agent opened the memory".
    pub fn is_read(&self) -> bool {
        matches!(self, SegmentSource::ToolRead)
    }

    /// Whether a hit here is authorship rather than reference.
    pub fn is_write(&self) -> bool {
        matches!(self, SegmentSource::ToolWrite)
    }
}

/// One piece of a session's transcript, as the scanner sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment<'a> {
    pub source: SegmentSource,
    pub text: &'a str,
}

impl<'a> Segment<'a> {
    pub fn new(source: SegmentSource, text: &'a str) -> Self {
        Self { source, text }
    }
}

/// A session's transcript reduced to what telemetry may look at.
///
/// Deliberately **not** `fartcode_acp::LiveModels`: ADR-0029 keeps ACP wire
/// and live-model types in the leaf crate, and this shape is also what a
/// PTY scrollback can be flattened into. The app layer adapts; this crate
/// stays free of both `fartcode-acp` and `fartcode-core`.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptView<'a> {
    pub segments: Vec<Segment<'a>>,
    /// Provider context-window usage, when the session reported any. This
    /// is the *only* usage metadata that reaches this process — see
    /// [`crate::tokens`] for what that does and does not permit.
    pub context_used: Option<u64>,
    pub context_size: Option<u64>,
    pub fidelity: Fidelity,
}

impl<'a> TranscriptView<'a> {
    /// An empty view for a session nothing could be read from. Scans over
    /// it yield [`Citation::Unknown`], never "not cited".
    pub fn absent() -> Self {
        Self {
            segments: Vec::new(),
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Absent,
        }
    }
}

/// The durable record of one settled step session.
///
/// Fixed size and text-free by design (see the module docs). Serde field
/// names are stable: this is persisted, and a rename would silently orphan
/// every stored row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepObservation {
    pub project_id: String,
    pub issue_id: String,
    /// The settling session's identity (`acp:<conversation>` /
    /// `pty:<terminal>`), the same string the step engine's launch registry
    /// uses.
    ///
    /// Present so an observation can be **replaced** rather than appended:
    /// the ACP hook fires on every turn that settles Done, and each of
    /// those scans the whole cumulative transcript, so appending would
    /// count one session once per turn. See [`ObservationLog::record`].
    pub session: String,
    /// The pipeline column the step ran as, recorded *before* an advancing
    /// settle moves the card off it.
    pub column: String,
    /// Unix seconds. Supplied by the app — this crate has no clock.
    pub settled_at: i64,
    pub citation: Citation,
    /// Whether this step **wrote** to the dossier without reading it.
    ///
    /// Surfaced rather than discarded because it is the size of a known
    /// confounder: the seeded prompt tells every step to append a section,
    /// so a corpus of write-only sessions is a corpus doing what it was
    /// told. A caller can see how much of the citation number is made of
    /// obedience.
    #[serde(default)]
    pub wrote_dossier: bool,
    pub reask: ReAskObservation,
    pub context_used: Option<u64>,
    pub context_size: Option<u64>,
    /// The fidelity the scan **settled on**, not the one it started with: a
    /// scan that ran out of budget over a `Full` transcript records
    /// `Truncated` here, and rates are computed over `Full` rows only.
    pub fidelity: Fidelity,
}

/// What a step's transcript said about clarifications — or that it could
/// not be asked.
///
/// The distinction mirrors [`Citation::Unknown`], and for the same reason:
/// "we scanned and it tagged nothing" and "we had nothing scannable to
/// look at" are different facts, and folding them together lets an
/// unreadable corpus masquerade as a well-behaved one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReAskObservation {
    /// Nothing scannable: no transcript, a truncated one, or a flattened
    /// scrollback whose provenance is gone.
    Unreadable,
    /// Scanned. The tally may be empty, which means this step recorded no
    /// clarification — a real observation, not an absence of one.
    Scanned(ReAskTally),
}

impl ReAskObservation {
    pub fn tally(&self) -> Option<ReAskTally> {
        match self {
            ReAskObservation::Unreadable => None,
            ReAskObservation::Scanned(tally) => Some(*tally),
        }
    }
}

/// How many observations one project keeps.
///
/// **The row cap, and why this number.** The log is a single versioned-JSON
/// kv value that is rewritten on every append, so its size is the cost of
/// every settle. At ~120 bytes per entry, 500 entries is a ~60 KiB rewrite
/// — under a millisecond, and far below the point where a settle would
/// notice. 500 settled steps is also several months of a busy project,
/// comfortably more than the dashboard's 90-day window
/// ([`crate::memory::DEFAULT_WINDOW_DAYS`]) asks for. Oldest entries are
/// evicted first, so the window the dashboard reads is always the fresh
/// end.
pub const MAX_OBSERVATIONS: usize = 500;

/// A project's bounded, append-only observation log.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationLog {
    pub entries: Vec<StepObservation>,
}

impl ObservationLog {
    /// Appends one observation, evicting the oldest entries past
    /// [`MAX_OBSERVATIONS`]. Returns how many were evicted, so the caller
    /// can report the log as clipped rather than let a truncated history
    /// read as a complete one.
    ///
    /// **One row is one settled step run**, and the identity is upstream:
    /// the app records only after the step engine's `begin_settle` returns
    /// `Act`, and the engine tombstones that entry, so the later turns of a
    /// chatty session no-op instead of arriving here. This used to
    /// de-duplicate on `(card, session)` and that was wrong in both
    /// directions — it merged two step runs that shared an identity (a
    /// task keeps ONE agent terminal for its whole life under ADR-0033, so
    /// `pty:<terminal>` is stable across every column the card passes
    /// through, and the second step silently overwrote the first) while
    /// still admitting turns the engine had already refused to settle.
    pub fn push(&mut self, observation: StepObservation) -> usize {
        self.entries.push(observation);
        let over = self.entries.len().saturating_sub(MAX_OBSERVATIONS);
        if over > 0 {
            self.entries.drain(..over);
        }
        over
    }

    /// Whether the log is at its cap — i.e. older observations have been
    /// dropped and any aggregate over it is a floor, not a total.
    pub fn is_full(&self) -> bool {
        self.entries.len() >= MAX_OBSERVATIONS
    }

    /// Observations for `project_id` settled at or after `since` (unix
    /// seconds).
    pub fn window(&self, project_id: &str, since: i64) -> Vec<&StepObservation> {
        self.entries
            .iter()
            .filter(|o| o.project_id == project_id && o.settled_at >= since)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(project: &str, at: i64) -> StepObservation {
        session_obs(project, "iss_1", "acp:c1", at)
    }

    fn session_obs(project: &str, issue: &str, session: &str, at: i64) -> StepObservation {
        StepObservation {
            project_id: project.into(),
            issue_id: issue.into(),
            session: session.into(),
            column: "Plan".into(),
            settled_at: at,
            citation: Citation::Unknown,
            wrote_dossier: false,
            reask: ReAskObservation::Unreadable,
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Absent,
        }
    }

    #[test]
    fn push_evicts_oldest_past_the_cap() {
        let mut log = ObservationLog::default();
        for i in 0..MAX_OBSERVATIONS as i64 {
            assert_eq!(log.push(obs("p1", i)), 0);
        }
        assert!(log.is_full());
        assert_eq!(log.push(obs("p1", 9_999)), 1);
        assert_eq!(log.entries.len(), MAX_OBSERVATIONS);
        // The oldest went, the newest stayed.
        assert_eq!(log.entries.first().unwrap().settled_at, 1);
        assert_eq!(log.entries.last().unwrap().settled_at, 9_999);
    }

    /// Two step runs on ONE agent terminal are two rows. ADR-0033 keeps one
    /// agent terminal per task for the card's whole life, so `pty:<id>` is
    /// the same string in every column — a store keyed on it would report
    /// a four-step feature as one step.
    #[test]
    fn two_runs_sharing_a_session_identity_are_two_rows() {
        let mut log = ObservationLog::default();
        let mut plan = session_obs("p1", "iss_1", "pty:t1", 100);
        plan.column = "Plan".into();
        let mut implement = session_obs("p1", "iss_1", "pty:t1", 200);
        implement.column = "Implement".into();
        log.push(plan);
        log.push(implement);
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].column, "Plan");
        assert_eq!(log.entries[1].column, "Implement");
    }

    #[test]
    fn window_filters_by_project_and_time() {
        let mut log = ObservationLog::default();
        log.push(obs("p1", 100));
        log.push(obs("p2", 200));
        log.push(obs("p1", 300));
        let got = log.window("p1", 150);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].settled_at, 300);
        assert!(log.window("p3", 0).is_empty());
    }

    #[test]
    fn an_absent_view_is_empty_and_says_so() {
        let view = TranscriptView::absent();
        assert!(view.segments.is_empty());
        assert_eq!(view.fidelity, Fidelity::Absent);
        assert_eq!(view.context_used, None);
    }

    /// The two exclusions, and the second is the one the PTY path needs:
    /// a flattened scrollback echoes the prompt, so nothing in it can be
    /// attributed.
    #[test]
    fn the_prompt_and_unprovenanced_text_are_never_scanned() {
        assert!(!SegmentSource::InjectedPrompt.scannable());
        assert!(!SegmentSource::Unstructured.scannable());
        for source in [
            SegmentSource::AgentProse,
            SegmentSource::ToolRead,
            SegmentSource::ToolWrite,
            SegmentSource::ToolExec,
            SegmentSource::Other,
        ] {
            assert!(source.scannable(), "{source:?}");
        }
    }

    #[test]
    fn an_unreadable_reask_observation_yields_no_tally() {
        assert_eq!(ReAskObservation::Unreadable.tally(), None);
        assert_eq!(
            ReAskObservation::Scanned(ReAskTally::default()).tally(),
            Some(ReAskTally::default())
        );
    }
}
