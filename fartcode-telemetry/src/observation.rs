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
    /// A shell command line and its output. Ambiguous by nature (`rg
    /// docs/features/` reads, `git add docs/features/x.md` writes), so it
    /// is scored at the weaker prose tier.
    ToolExec,
    /// Anything else, including a flattened PTY scrollback with no
    /// structure to read.
    Other,
}

impl SegmentSource {
    /// Whether this segment is scanned at all.
    pub fn scannable(&self) -> bool {
        !matches!(self, SegmentSource::InjectedPrompt)
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
    pub reask: ReAskTally,
    pub context_used: Option<u64>,
    pub context_size: Option<u64>,
    pub fidelity: Fidelity,
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
    /// Records one observation, **replacing** any earlier one for the same
    /// card and session, and evicting the oldest entries past
    /// [`MAX_OBSERVATIONS`]. Returns how many were evicted, so the caller
    /// can report the log as clipped rather than let a truncated history
    /// read as a complete one.
    ///
    /// The replace is what makes the capture idempotent. A settling ACP
    /// session fires the hook once per turn that finishes Done, and every
    /// one of those scans the whole transcript so far — appending them
    /// would enter the same session three or ten times, each a superset of
    /// the last, and the citation rate would silently weight chatty
    /// sessions. Replacing keeps exactly one row per session: the last, and
    /// therefore the most complete.
    pub fn record(&mut self, observation: StepObservation) -> usize {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.issue_id == observation.issue_id && e.session == observation.session)
        {
            *existing = observation;
            return 0;
        }
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
        let mut observation = session_obs(project, "iss_1", "acp:c1", at);
        // Distinct sessions, so the eviction tests exercise the append path
        // rather than the replace path.
        observation.session = format!("acp:{at}");
        observation
    }

    fn session_obs(project: &str, issue: &str, session: &str, at: i64) -> StepObservation {
        StepObservation {
            project_id: project.into(),
            issue_id: issue.into(),
            session: session.into(),
            column: "Plan".into(),
            settled_at: at,
            citation: Citation::Unknown,
            reask: ReAskTally::default(),
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Absent,
        }
    }

    #[test]
    fn record_evicts_oldest_past_the_cap() {
        let mut log = ObservationLog::default();
        for i in 0..MAX_OBSERVATIONS as i64 {
            assert_eq!(log.record(obs("p1", i)), 0);
        }
        assert!(log.is_full());
        assert_eq!(log.record(obs("p1", 9_999)), 1);
        assert_eq!(log.entries.len(), MAX_OBSERVATIONS);
        // The oldest went, the newest stayed.
        assert_eq!(log.entries.first().unwrap().settled_at, 1);
        assert_eq!(log.entries.last().unwrap().settled_at, 9_999);
    }

    /// One session is one row however many times its turns settle.
    #[test]
    fn recording_the_same_session_twice_replaces_rather_than_appends() {
        let mut log = ObservationLog::default();
        log.record(session_obs("p1", "iss_1", "acp:c1", 100));
        log.record(session_obs("p1", "iss_1", "acp:c1", 200));
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].settled_at, 200);

        // A different session, or a different card, is a different row.
        log.record(session_obs("p1", "iss_1", "acp:c2", 300));
        log.record(session_obs("p1", "iss_2", "acp:c1", 400));
        assert_eq!(log.entries.len(), 3);
    }

    #[test]
    fn window_filters_by_project_and_time() {
        let mut log = ObservationLog::default();
        log.record(obs("p1", 100));
        log.record(obs("p2", 200));
        log.record(obs("p1", 300));
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

    #[test]
    fn the_injected_prompt_is_never_scanned() {
        assert!(!SegmentSource::InjectedPrompt.scannable());
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
}
