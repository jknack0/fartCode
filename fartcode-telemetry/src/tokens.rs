//! Signal 3 — **context tokens saved**: referencing the dossier instead of
//! re-deriving what is in it.
//!
//! ADR-0038 says the arithmetic comes "from provider usage metadata the
//! transcript reducer already sees". Here is exactly what that metadata is,
//! because it constrains the answer completely:
//!
//! ```text
//! SessionUsage { context_size: u64, context_used: u64, cost: Option<UsageCost> }
//! ```
//!
//! That is the ACP `usage_update` frame (`fartcode-acp`), and it is a
//! **context-window gauge**, not token accounting. There are no input,
//! output, or cache-read counts anywhere in this workspace — the ACP wire
//! schema does not carry them, so the provider's real token bill never
//! reaches this process. `context_used` is also last-write-wins in the
//! reducer: a high-water reading at the end of the session, not a sum over
//! turns.
//!
//! # What can honestly be computed from that
//!
//! Not a saving, in the sense of "tokens that were not spent". There is no
//! counterfactual: the re-derivation that did not happen has no cost we
//! observed. What there *is* is a comparison — sessions in this project
//! that cited a dossier against sessions that did not — and a difference of
//! medians between them.
//!
//! So the type is [`TokensSaved::Estimated`], the payload is
//! [`EstimatedTokens`], and [`EstimatedTokens::label`] says "estimated"
//! along with its sample size. The word is in the type name, the variant
//! name, and the rendered string, so #76 cannot surface the figure without
//! it — the same posture as [`crate::time_to_land`], for the same reason.
//!
//! # The confounders, named
//!
//! - **Task difficulty is not controlled.** A step that consults memory may
//!   be a step with less to do. The contrast is observational, not an
//!   experiment.
//! - **Session length dominates context use.** A long session fills the
//!   window regardless of whether it read a dossier.
//! - **The gauge is a high-water mark**, so a session that compacted its
//!   context reads lower than it cost.
//! - **The difference can be negative** — citing sessions may use *more*
//!   context, because reading a file puts it in the window. That is
//!   reported as a negative number, never clamped to zero. A metric that
//!   can only be flattering is not a measurement.

use serde::{Deserialize, Serialize};

use crate::observation::StepObservation;

/// What the numbers are derived from. One variant today; it exists so the
/// basis travels with the figure and a future provider that really does
/// report token counts is a new variant rather than a silent meaning
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenBasis {
    /// ACP `usage_update` — a context-window gauge sampled at the end of a
    /// session. Not a billing figure and not a per-turn total.
    ContextWindowGauge,
}

impl TokenBasis {
    pub fn describe(&self) -> &'static str {
        match self {
            TokenBasis::ContextWindowGauge => {
                "context-window gauge (ACP usage_update), not a billing figure"
            }
        }
    }
}

/// Fewest sessions per arm before a median means anything.
///
/// Three, not two: with two readings the "median" is their mean and one
/// unusual session moves it by half its own size. Three is still a small
/// sample — [`EstimatedTokens::label`] always prints `n` so nobody reads
/// the figure as settled — but it is the point below which the number is
/// noise wearing a statistic's clothes.
pub const MIN_SAMPLE_PER_ARM: usize = 3;

/// A context contrast between citing and non-citing sessions. Every field
/// is an estimate; the name says so once and the label says so again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimatedTokens {
    /// Median context tokens a non-citing session ended on, minus the
    /// median a citing one did. **May be negative** — see the module docs.
    pub per_session: i64,
    /// `per_session` times the number of citing sessions in the window.
    /// The headline figure, and the one most in need of its label.
    pub window_total: i64,
    pub citing_median: u64,
    pub not_citing_median: u64,
    pub citing_sessions: u32,
    pub not_citing_sessions: u32,
    pub basis: TokenBasis,
}

impl EstimatedTokens {
    /// The figure as it should be shown: never a bare number.
    pub fn label(&self) -> String {
        let direction = if self.window_total >= 0 {
            "lower"
        } else {
            "higher"
        };
        format!(
            "≈{} context tokens {direction} across {} citing session(s) — ESTIMATED from a \
             {} vs {} session comparison of {}; observational, not a controlled saving",
            self.window_total.abs(),
            self.citing_sessions,
            self.citing_sessions,
            self.not_citing_sessions,
            self.basis.describe(),
        )
    }
}

/// The signal. `Insufficient` is the normal answer for a young project and
/// is not a zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TokensSaved {
    /// Not enough sessions with usage metadata on both sides of the
    /// contrast. Carries what it has so the dashboard can say how far off
    /// it is rather than draw a zero.
    Insufficient {
        citing_with_usage: u32,
        not_citing_with_usage: u32,
        needed_per_arm: u32,
    },
    Estimated(EstimatedTokens),
}

impl TokensSaved {
    pub fn label(&self) -> String {
        match self {
            TokensSaved::Insufficient {
                citing_with_usage,
                not_citing_with_usage,
                needed_per_arm,
            } => format!(
                "not enough data — {citing_with_usage} citing and {not_citing_with_usage} \
                 non-citing session(s) reported usage; {needed_per_arm} of each are needed \
                 before a comparison means anything"
            ),
            TokensSaved::Estimated(estimate) => estimate.label(),
        }
    }
}

/// Builds the contrast from a window of observations.
///
/// Only [`crate::citations::Citation::is_conclusive`] sessions take part: an
/// `Unknown`
/// citation cannot be put on either side, and guessing would put the
/// confounder into the arm assignment itself.
pub fn estimate(observations: &[&StepObservation]) -> TokensSaved {
    let mut citing: Vec<u64> = Vec::new();
    let mut not_citing: Vec<u64> = Vec::new();

    for observation in observations {
        let Some(used) = observation.context_used else {
            continue;
        };
        if !observation.citation.is_conclusive() {
            continue;
        }
        if observation.citation.is_cited() {
            citing.push(used);
        } else {
            not_citing.push(used);
        }
    }

    if citing.len() < MIN_SAMPLE_PER_ARM || not_citing.len() < MIN_SAMPLE_PER_ARM {
        return TokensSaved::Insufficient {
            citing_with_usage: citing.len() as u32,
            not_citing_with_usage: not_citing.len() as u32,
            needed_per_arm: MIN_SAMPLE_PER_ARM as u32,
        };
    }

    let citing_median = median(&mut citing);
    let not_citing_median = median(&mut not_citing);
    let per_session = i64::try_from(not_citing_median).unwrap_or(i64::MAX)
        - i64::try_from(citing_median).unwrap_or(i64::MAX);

    TokensSaved::Estimated(EstimatedTokens {
        per_session,
        window_total: per_session.saturating_mul(citing.len() as i64),
        citing_median,
        not_citing_median,
        citing_sessions: citing.len() as u32,
        not_citing_sessions: not_citing.len() as u32,
        basis: TokenBasis::ContextWindowGauge,
    })
}

/// Median of a non-empty slice. Even counts take the lower-mean of the two
/// middles (integer mean, rounded down) — no float round-trip on token
/// counts.
fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        // Averaged without overflowing: a + (b - a) / 2.
        let (a, b) = (values[n / 2 - 1], values[n / 2]);
        a + (b - a) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citations::Citation;
    use crate::observation::Fidelity;
    use crate::reask::ReAskTally;

    fn obs(citation: Citation, used: Option<u64>) -> StepObservation {
        StepObservation {
            project_id: "p1".into(),
            issue_id: "iss_1".into(),
            session: "acp:c1".into(),
            column: "Plan".into(),
            settled_at: 0,
            citation,
            wrote_dossier: false,
            reask: crate::observation::ReAskObservation::Scanned(ReAskTally::default()),
            context_used: used,
            context_size: Some(200_000),
            fidelity: Fidelity::Full,
        }
    }

    fn estimate_of(all: &[StepObservation]) -> TokensSaved {
        let refs: Vec<&StepObservation> = all.iter().collect();
        estimate(&refs)
    }

    #[test]
    fn a_known_contrast_yields_the_expected_estimate() {
        let all = vec![
            obs(Citation::CitedRead, Some(10_000)),
            obs(Citation::CitedRead, Some(12_000)),
            obs(Citation::CitedMention, Some(14_000)),
            obs(Citation::NotCited, Some(30_000)),
            obs(Citation::NotCited, Some(32_000)),
            obs(Citation::NotCited, Some(34_000)),
        ];
        let TokensSaved::Estimated(estimate) = estimate_of(&all) else {
            panic!("expected an estimate");
        };
        assert_eq!(estimate.citing_median, 12_000);
        assert_eq!(estimate.not_citing_median, 32_000);
        assert_eq!(estimate.per_session, 20_000);
        assert_eq!(estimate.window_total, 60_000);
        assert_eq!(estimate.citing_sessions, 3);
        assert_eq!(estimate.basis, TokenBasis::ContextWindowGauge);
        // The uncertainty is in the surfaced value, not only in a comment.
        assert!(
            estimate.label().contains("ESTIMATED"),
            "{}",
            estimate.label()
        );
        assert!(estimate.label().contains("not a billing figure"));
    }

    /// A metric that can only flatter itself is not a measurement.
    #[test]
    fn citing_sessions_using_more_context_reports_a_negative_number() {
        let all = vec![
            obs(Citation::CitedRead, Some(40_000)),
            obs(Citation::CitedRead, Some(42_000)),
            obs(Citation::CitedRead, Some(44_000)),
            obs(Citation::NotCited, Some(10_000)),
            obs(Citation::NotCited, Some(11_000)),
            obs(Citation::NotCited, Some(12_000)),
        ];
        let TokensSaved::Estimated(estimate) = estimate_of(&all) else {
            panic!("expected an estimate");
        };
        assert_eq!(estimate.per_session, -31_000);
        assert!(estimate.window_total < 0);
        assert!(estimate.label().contains("higher"));
    }

    #[test]
    fn too_few_sessions_is_insufficient_not_zero() {
        let all = vec![
            obs(Citation::CitedRead, Some(10_000)),
            obs(Citation::NotCited, Some(30_000)),
        ];
        assert_eq!(
            estimate_of(&all),
            TokensSaved::Insufficient {
                citing_with_usage: 1,
                not_citing_with_usage: 1,
                needed_per_arm: MIN_SAMPLE_PER_ARM as u32,
            }
        );
        assert!(estimate_of(&all).label().contains("not enough data"));
    }

    #[test]
    fn no_observations_at_all_is_insufficient() {
        assert_eq!(
            estimate_of(&[]),
            TokensSaved::Insufficient {
                citing_with_usage: 0,
                not_citing_with_usage: 0,
                needed_per_arm: MIN_SAMPLE_PER_ARM as u32,
            }
        );
    }

    #[test]
    fn sessions_without_usage_or_with_an_unknown_verdict_take_no_side() {
        let all = vec![
            obs(Citation::CitedRead, Some(10_000)),
            obs(Citation::CitedRead, Some(10_000)),
            obs(Citation::CitedRead, Some(10_000)),
            // No usage metadata (a PTY session): excluded entirely.
            obs(Citation::NotCited, None),
            // Unknown verdict: cannot be assigned to an arm.
            obs(Citation::Unknown, Some(99_000)),
            obs(Citation::NotCited, Some(20_000)),
            obs(Citation::NotCited, Some(20_000)),
        ];
        assert_eq!(
            estimate_of(&all),
            TokensSaved::Insufficient {
                citing_with_usage: 3,
                not_citing_with_usage: 2,
                needed_per_arm: 3,
            }
        );
    }

    #[test]
    fn median_handles_even_and_odd_lengths() {
        assert_eq!(median(&mut [5]), 5);
        assert_eq!(median(&mut [9, 1, 5]), 5);
        assert_eq!(median(&mut [4, 2]), 3);
        assert_eq!(median(&mut [u64::MAX, u64::MAX - 2]), u64::MAX - 1);
    }
}
