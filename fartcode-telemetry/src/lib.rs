//! fartcode-telemetry — local memory-value signals (E19-04, #73;
//! ADR-0038 item 7).
//!
//! ADR-0038 item 7 asks this crate for four numbers about whether feature
//! dossiers are worth their cost: **memory citations**, **re-ask rate**,
//! **context tokens saved**, and **time-to-land trend**. The ADR is equally
//! specific about where they may go: "in-app only", "all local — no metric
//! leaves the machine".
//!
//! # The no-egress contract
//!
//! This crate has no network dependency, no filesystem access, no database
//! handle, and no clock. It is a **pure function library**: the app hands it
//! values, it hands back values. There is no code path along which a number
//! computed here could reach a wire, because there is nothing here that can
//! open one. `tests/no_egress.rs` re-checks that against the manifest and
//! the sources on every `cargo test`, so a future edit that adds `reqwest`
//! "just for an opt-in share" fails the build.
//!
//! Purity is also what makes the signals testable: every one of the four is
//! a fold over a fixture with a known right answer, no app instance needed.
//!
//! # Honesty is in the types, not the comments
//!
//! All four signals are heuristics, and a metric that flatters itself is
//! worse than no metric — the whole point of item 7 is to let the feature
//! prove or disprove its own value. So uncertainty is encoded where a
//! consumer must handle it:
//!
//! - [`citations::Citation::Unknown`] — the transcript was truncated (a PTY
//!   scrollback ring) and its silence proves nothing. Never folded into
//!   "not cited".
//! - [`reask::ReAskRate::Unknown`] — nothing emitted the tag. Not 0%, not
//!   100%.
//! - [`tokens::TokensSaved::Estimated`] — the only provider metadata that
//!   reaches this process is a context-window gauge, so the figure is an
//!   estimate by construction and says so in its own name and label.
//! - [`time_to_land::TimeToLand`] — the numbers are private and reachable
//!   only together with the caveat ADR-0038 forbids omitting.
//!
//! # What the app layer supplies
//!
//! Transcripts are not persisted anywhere in fartCode (ADR-0029 item 5: the
//! raw ACP log is in-memory and dies with the process; the `messages` table
//! has been inert since Phase 0). So signals 1–3 cannot be recomputed later
//! from stored text — they are scanned **while the transcript is still in
//! memory**, at settle, and only the small fixed-size verdict
//! ([`observation::StepObservation`]) is kept. Signal 4 needs no capture at
//! all: `## Timeline` breadcrumbs are in the repository.
//!
//! `fartcode_app::telemetry` owns the capture hooks, the store, and the
//! on-demand aggregation command; this crate owns the meaning.

pub mod citations;
pub mod memory;
pub mod observation;
pub mod reask;
pub mod time_to_land;
pub mod tokens;

pub use citations::{Citation, CitationScan, CitationTargets};
pub use memory::{Citations, MemoryInputs, MemoryValue};
pub use observation::{Fidelity, ObservationLog, Segment, SegmentSource, StepObservation};
pub use reask::{ReAskRate, ReAskTally};
pub use time_to_land::{FeatureCycle, TimeToLand, TimeToLandKind, TimelineEvent, TREND_CAVEAT};
pub use tokens::{EstimatedTokens, TokenBasis, TokensSaved};
