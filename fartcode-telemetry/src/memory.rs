//! The four signals, folded into one value for the dashboard (#76).
//!
//! Assembly only — each signal's meaning lives in its own module. What is
//! decided here is the shape of the answer, and one thing the ADR does not
//! spell out: **every count that feeds a rate is reported next to it**, so a
//! consumer that wants to render "83%" has the `5 of 6` sitting right
//! there. A rate over six sessions and a rate over six hundred are not the
//! same claim, and the payload should not let them look alike.

use serde::Serialize;

use crate::citations::Citation;
use crate::observation::StepObservation;
use crate::reask::{ReAskRate, ReAskTally};
use crate::time_to_land::{FeatureCycle, TimeToLand};
use crate::tokens::{estimate, TokensSaved};

/// How far back the dashboard looks by default.
///
/// **The time bound, and why this number.** ADR-0038 frames the surface as
/// "your project memory saved N re-explanations *this month*", so the
/// window is a recency claim, not a lifetime total. 90 days is three of
/// those months: long enough that a project shipping a feature a fortnight
/// holds enough landings for a two-sided time-to-land split, short enough
/// that a pipeline change a year ago is not still shaping the headline. It
/// also keeps the fold well inside the row cap
/// ([`crate::observation::MAX_OBSERVATIONS`]), so the window is normally
/// the binding constraint and the cap is the backstop.
pub const DEFAULT_WINDOW_DAYS: u32 = 90;

/// The citation signal, with its denominators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citations {
    pub sessions: u32,
    /// A read or search tool call resolved the dossier.
    pub cited_read: u32,
    /// Named in prose or a shell line only.
    pub cited_mention: u32,
    /// Whole transcript in hand, no reference.
    pub not_cited: u32,
    /// Nothing could be concluded — no transcript, or only a truncated
    /// tail with no hit. Reported beside the rate, never inside it.
    pub unknown: u32,
}

impl Citations {
    pub fn tally<'a>(verdicts: impl IntoIterator<Item = &'a Citation>) -> Self {
        let mut out = Citations::default();
        for verdict in verdicts {
            out.sessions += 1;
            match verdict {
                Citation::CitedRead => out.cited_read += 1,
                Citation::CitedMention => out.cited_mention += 1,
                Citation::NotCited => out.not_cited += 1,
                Citation::Unknown => out.unknown += 1,
            }
        }
        out
    }

    /// Sessions that cited, over sessions where a verdict was possible.
    /// `None` when nothing was conclusive — an all-`Unknown` window has no
    /// rate, rather than a rate of zero.
    pub fn rate(&self) -> Option<f64> {
        let conclusive = self.cited_read + self.cited_mention + self.not_cited;
        (conclusive > 0)
            .then(|| f64::from(self.cited_read + self.cited_mention) / f64::from(conclusive))
    }

    pub fn label(&self) -> String {
        match self.rate() {
            None => format!(
                "unknown — none of the {} session(s) in this window left a readable transcript",
                self.sessions
            ),
            Some(rate) => {
                let cited = self.cited_read + self.cited_mention;
                let conclusive = cited + self.not_cited;
                let mut out = format!(
                    "{:.0}% of steps read their feature's memory ({cited} of {conclusive}; \
                     {} through a read or search, {} named it only)",
                    rate * 100.0,
                    self.cited_read,
                    self.cited_mention,
                );
                if self.unknown > 0 {
                    out.push_str(&format!(
                        " — {} further session(s) were unreadable and are excluded",
                        self.unknown
                    ));
                }
                out
            }
        }
    }
}

/// Everything [`compute`] needs. The app layer gathers it; this crate does
/// no I/O.
#[derive(Debug, Clone)]
pub struct MemoryInputs<'a> {
    pub project_id: &'a str,
    pub window_days: u32,
    /// Settled-step observations inside the window.
    pub observations: &'a [&'a StepObservation],
    /// Re-ask tags found in committed dossier sections. The durable half of
    /// the convention: a transcript is gone by morning, a dossier section is
    /// in git.
    pub dossier_tallies: &'a [ReAskTally],
    /// One entry per feature with both a `created` and a `pr merged`
    /// breadcrumb.
    pub cycles: Vec<FeatureCycle>,
    /// How many dossiers were read for `cycles` and `dossier_tallies`.
    pub dossiers_scanned: u32,
    /// True when a bound clipped the input — the observation log was at its
    /// row cap, or more dossiers exist than were read. Aggregates are then
    /// floors, and the dashboard should say so.
    pub clipped: bool,
}

/// The dashboard payload. Local, and there is nothing in this crate that
/// could make it otherwise.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryValue {
    pub project_id: String,
    pub window_days: u32,
    pub citations: Citations,
    pub re_ask: ReAskRate,
    pub tokens_saved: TokensSaved,
    pub time_to_land: TimeToLand,
    pub sessions_observed: u32,
    pub dossiers_scanned: u32,
    /// See [`MemoryInputs::clipped`].
    pub clipped: bool,
}

impl MemoryValue {
    /// Four lines, each already carrying its own uncertainty. Handy for a
    /// log line, and the reference rendering #76 can diverge from
    /// knowingly.
    pub fn lines(&self) -> [String; 4] {
        [
            format!("citations: {}", self.citations.label()),
            format!("re-ask: {}", self.re_ask.label()),
            format!("context: {}", self.tokens_saved.label()),
            format!("time to land: {}", self.time_to_land.summary()),
        ]
    }
}

/// Folds the four signals. Pure: same inputs, same answer, no clock.
pub fn compute(inputs: MemoryInputs<'_>) -> MemoryValue {
    let citations = Citations::tally(inputs.observations.iter().map(|o| &o.citation));

    // Both halves of the re-ask convention, in one fold: the per-session
    // tallies scanned while the transcript existed, and the tallies still
    // sitting in committed dossier sections. A step that emitted the tag
    // in both places is counted twice — deliberate: the two are different
    // clarifications' worth of evidence only when they differ, and
    // deduplicating them would need the question text, which is exactly
    // the content this crate refuses to store.
    let tallies: Vec<ReAskTally> = inputs
        .observations
        .iter()
        .map(|o| o.reask)
        .chain(inputs.dossier_tallies.iter().copied())
        .collect();
    let re_ask = ReAskRate::from_tallies(tallies.iter());

    MemoryValue {
        project_id: inputs.project_id.to_string(),
        window_days: inputs.window_days,
        citations,
        re_ask,
        tokens_saved: estimate(inputs.observations),
        time_to_land: TimeToLand::from_cycles(inputs.cycles),
        sessions_observed: inputs.observations.len() as u32,
        dossiers_scanned: inputs.dossiers_scanned,
        clipped: inputs.clipped,
    }
}

/// An empty report for a project with nothing to say yet. Every signal
/// lands on its own "not enough information" variant, which is the point:
/// a fresh project renders four honest blanks, not four zeroes.
pub fn empty(project_id: &str, window_days: u32) -> MemoryValue {
    compute(MemoryInputs {
        project_id,
        window_days,
        observations: &[],
        dossier_tallies: &[],
        cycles: Vec::new(),
        dossiers_scanned: 0,
        clipped: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Fidelity;
    use crate::time_to_land::{TimeToLandKind, TREND_CAVEAT};
    use crate::tokens::MIN_SAMPLE_PER_ARM;

    fn obs(citation: Citation, used: Option<u64>, reask: ReAskTally) -> StepObservation {
        StepObservation {
            project_id: "p1".into(),
            issue_id: "iss_1".into(),
            session: "acp:c1".into(),
            column: "Plan".into(),
            settled_at: 0,
            citation,
            reask,
            context_used: used,
            context_size: Some(200_000),
            fidelity: Fidelity::Full,
        }
    }

    /// No dossiers, no transcripts, no cycles: four honest blanks.
    #[test]
    fn an_empty_project_reports_unknowns_rather_than_zeroes() {
        let value = empty("p1", DEFAULT_WINDOW_DAYS);
        assert_eq!(value.citations.rate(), None);
        assert_eq!(value.re_ask.rate(), None);
        assert_eq!(
            value.tokens_saved,
            TokensSaved::Insufficient {
                citing_with_usage: 0,
                not_citing_with_usage: 0,
                needed_per_arm: MIN_SAMPLE_PER_ARM as u32,
            }
        );
        assert_eq!(value.time_to_land.read().0, TimeToLandKind::NoData);
        assert_eq!(value.sessions_observed, 0);
        // Every line still carries its uncertainty.
        for line in value.lines() {
            assert!(!line.is_empty());
        }
        assert!(value.lines()[3].contains(TREND_CAVEAT));
    }

    #[test]
    fn a_constructed_window_folds_to_the_known_answer() {
        let all = [
            obs(
                Citation::CitedRead,
                Some(10_000),
                ReAskTally {
                    memory_answered: 2,
                    human_asked: 0,
                },
            ),
            obs(Citation::CitedRead, Some(12_000), ReAskTally::default()),
            obs(Citation::CitedMention, Some(14_000), ReAskTally::default()),
            obs(Citation::NotCited, Some(30_000), ReAskTally::default()),
            obs(Citation::NotCited, Some(32_000), ReAskTally::default()),
            obs(Citation::NotCited, Some(34_000), ReAskTally::default()),
            obs(Citation::Unknown, None, ReAskTally::default()),
        ];
        let refs: Vec<&StepObservation> = all.iter().collect();
        let dossier_tallies = [ReAskTally {
            memory_answered: 1,
            human_asked: 1,
        }];
        let value = compute(MemoryInputs {
            project_id: "p1",
            window_days: 30,
            observations: &refs,
            dossier_tallies: &dossier_tallies,
            // Landing order, not creation order, is what the trend splits
            // on: the slow one landed first.
            cycles: vec![
                FeatureCycle {
                    created: 0,
                    landed: 100 * 3_600,
                },
                FeatureCycle {
                    created: 400 * 3_600,
                    landed: 420 * 3_600,
                },
            ],
            dossiers_scanned: 2,
            clipped: false,
        });

        assert_eq!(
            value.citations,
            Citations {
                sessions: 7,
                cited_read: 2,
                cited_mention: 1,
                not_cited: 3,
                unknown: 1,
            }
        );
        // 3 cited of 6 conclusive — the Unknown session is excluded, not
        // counted as a miss.
        assert_eq!(value.citations.rate(), Some(0.5));

        // 1 human of 4 tagged clarifications (2 memory from a session, 1+1
        // from the dossier section).
        assert_eq!(value.re_ask.rate(), Some(0.25));

        let TokensSaved::Estimated(estimate) = value.tokens_saved else {
            panic!("expected an estimate");
        };
        assert_eq!(estimate.per_session, 20_000);

        assert_eq!(
            value.time_to_land.read().0,
            TimeToLandKind::Trend {
                earlier_median_hours: 100.0,
                later_median_hours: 20.0,
                landed: 2,
            }
        );
        assert_eq!(value.sessions_observed, 7);
        assert_eq!(value.dossiers_scanned, 2);
    }

    #[test]
    fn an_all_unknown_window_has_no_citation_rate() {
        let all = vec![obs(Citation::Unknown, None, ReAskTally::default()); 4];
        let refs: Vec<&StepObservation> = all.iter().collect();
        let value = compute(MemoryInputs {
            project_id: "p1",
            window_days: 90,
            observations: &refs,
            dossier_tallies: &[],
            cycles: Vec::new(),
            dossiers_scanned: 0,
            clipped: false,
        });
        assert_eq!(value.citations.unknown, 4);
        assert_eq!(value.citations.rate(), None);
        assert!(value.citations.label().contains("unknown"));
    }

    #[test]
    fn the_payload_always_carries_the_time_to_land_caveat() {
        let json = serde_json::to_string(&empty("p1", 90)).unwrap();
        assert!(json.contains("Trend, not attribution"), "{json}");
    }
}
