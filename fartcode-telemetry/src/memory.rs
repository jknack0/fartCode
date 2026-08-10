//! The four signals, folded into one value for the dashboard (#76).
//!
//! Assembly only — each signal's meaning lives in its own module. Two
//! things are decided here.
//!
//! **Every count that feeds a rate is reported next to it**, so a consumer
//! that wants to render "83%" has the `5 of 6` sitting right there. A rate
//! over six sessions and a rate over six hundred are not the same claim,
//! and the payload should not let them look alike.
//!
//! **The window bounds the whole report, not the half that was easy.**
//! Observations carry a settle timestamp and were always filtered; the
//! dossier-derived halves (time-to-land, and the re-ask tags committed in
//! agent sections) are filtered here, by landing time and by section date
//! respectively. A dossier section with no parseable date is **excluded and
//! counted**, not quietly included: a report labelled "the last 90 days"
//! that actually spans all history is a wrong number, not a rough one.

use serde::Serialize;

use crate::citations::Citation;
use crate::observation::{Fidelity, ReAskObservation, StepObservation};
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

/// One agent-written dossier section's tags, with the date its heading
/// carried. `at: None` means the heading had no parseable date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatedTally {
    pub at: Option<i64>,
    pub tally: ReAskTally,
}

/// The citation signal, with its denominators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citations {
    pub sessions: u32,
    /// A read or search tool call resolved the dossier.
    pub cited_read: u32,
    /// Named in agent prose only.
    pub cited_mention: u32,
    /// Whole transcript in hand, no reference.
    pub not_cited: u32,
    /// Sessions excluded from the rate: nothing readable, only a truncated
    /// tail, or a scan that ran out of budget.
    ///
    /// **Including the ones that showed a hit** — see `unknown_with_hit`.
    /// A session that can contribute a positive but could never have
    /// contributed a negative is a sample that can only flatter, so it
    /// stays out of both sides.
    pub unknown: u32,
    /// Of the excluded, how many nonetheless named the dossier. Kept so the
    /// information is not destroyed, and deliberately not in the rate.
    pub unknown_with_hit: u32,
    /// Rate-eligible sessions that wrote to the dossier without ever
    /// reading it.
    ///
    /// The size of a known confounder, surfaced rather than buried: the
    /// seeded prompt tells every step to append a section, so this is how
    /// much of the corpus is doing what it was told. It is not subtracted
    /// from anything — it is context for reading the number.
    pub wrote_without_reading: u32,
}

impl Citations {
    /// Folds one `(verdict, fidelity, wrote_dossier)` triple per session.
    ///
    /// Only [`Fidelity::Full`] sessions are rate-eligible.
    pub fn tally(sessions: impl IntoIterator<Item = (Citation, Fidelity, bool)>) -> Self {
        let mut out = Citations::default();
        for (verdict, fidelity, wrote) in sessions {
            out.sessions += 1;
            if fidelity != Fidelity::Full {
                out.unknown += 1;
                if verdict.is_cited() {
                    out.unknown_with_hit += 1;
                }
                continue;
            }
            match verdict {
                Citation::CitedRead => out.cited_read += 1,
                Citation::CitedMention => out.cited_mention += 1,
                Citation::NotCited => {
                    out.not_cited += 1;
                    if wrote {
                        out.wrote_without_reading += 1;
                    }
                }
                // A `Full` transcript does not produce `Unknown`, but the
                // arm is real rather than unreachable: it keeps a future
                // fidelity variant from silently landing in `not_cited`.
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
                "unknown — none of the {} session(s) in this window left a transcript that \
                 could be attributed",
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
                if self.wrote_without_reading > 0 {
                    out.push_str(&format!(
                        " — {} of the rest wrote their section without reading the file, which \
                         is what the step prompt asks for",
                        self.wrote_without_reading
                    ));
                }
                if self.unknown > 0 {
                    out.push_str(&format!(
                        "; {} further session(s) could not be attributed and are excluded ({} of \
                         them did name it)",
                        self.unknown, self.unknown_with_hit
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
    /// Start of the window, unix seconds. Applied to **every** signal here,
    /// not only to the observations the app pre-filtered.
    pub since: i64,
    /// Settled-step observations inside the window.
    pub observations: &'a [&'a StepObservation],
    /// Re-ask tags found in committed dossier sections, each with the date
    /// its heading carried. The durable half of the convention: a
    /// transcript is gone by morning, a dossier section is in git.
    pub dossier_tallies: &'a [DatedTally],
    /// One entry per feature with both a `created` and a `pr merged`
    /// breadcrumb. Filtered here by landing time.
    pub cycles: Vec<FeatureCycle>,
    /// How many dossiers were read for `cycles` and `dossier_tallies`.
    pub dossiers_scanned: u32,
    /// True when a bound clipped the input — the observation log was at its
    /// row cap, or more dossiers exist than were read.
    pub clipped: bool,
}

/// The dashboard payload. Local, and there is nothing in this crate that
/// could make it otherwise.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryValue {
    pub project_id: String,
    pub window_days: u32,
    /// Start of the window, unix seconds — so a consumer can state the
    /// range rather than infer it.
    pub window_since: i64,
    pub citations: Citations,
    pub re_ask: ReAskRate,
    pub tokens_saved: TokensSaved,
    pub time_to_land: TimeToLand,
    pub sessions_observed: u32,
    pub dossiers_scanned: u32,
    /// Landed features dropped for falling outside the window.
    pub cycles_outside_window: u32,
    /// Agent sections dropped because their heading carried no parseable
    /// date, so they could not be placed inside the window.
    pub sections_undated: u32,
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
    let citations = Citations::tally(
        inputs
            .observations
            .iter()
            .map(|o| (o.citation, o.fidelity, o.wrote_dossier)),
    );

    // Both halves of the re-ask convention, in one fold: the per-session
    // observations taken while the transcript existed, and the tags still
    // sitting in committed dossier sections. A step that emitted the tag in
    // both places is counted twice — deliberate: deduplicating them would
    // need the question text, which is exactly the content this crate
    // refuses to store.
    let mut sections_undated = 0u32;
    let dossier: Vec<ReAskObservation> = inputs
        .dossier_tallies
        .iter()
        .filter(|dated| match dated.at {
            Some(at) => at >= inputs.since,
            None => {
                sections_undated += 1;
                false
            }
        })
        .map(|dated| ReAskObservation::Scanned(dated.tally))
        .collect();
    let re_ask =
        ReAskRate::from_observations(inputs.observations.iter().map(|o| o.reask).chain(dossier));

    let landed_in_window: Vec<FeatureCycle> = inputs
        .cycles
        .iter()
        .copied()
        .filter(|cycle| cycle.landed >= inputs.since)
        .collect();
    let cycles_outside_window = (inputs.cycles.len() - landed_in_window.len()) as u32;

    MemoryValue {
        project_id: inputs.project_id.to_string(),
        window_days: inputs.window_days,
        window_since: inputs.since,
        citations,
        re_ask,
        tokens_saved: estimate(inputs.observations),
        time_to_land: TimeToLand::from_cycles(landed_in_window),
        sessions_observed: inputs.observations.len() as u32,
        dossiers_scanned: inputs.dossiers_scanned,
        cycles_outside_window,
        sections_undated,
        clipped: inputs.clipped,
    }
}

/// An empty report for a project with nothing to say yet. Every signal
/// lands on its own "not enough information" variant, which is the point:
/// a fresh project renders four honest blanks, not four zeroes.
pub fn empty(project_id: &str, window_days: u32, since: i64) -> MemoryValue {
    compute(MemoryInputs {
        project_id,
        window_days,
        since,
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
    use crate::time_to_land::{TimeToLandKind, TREND_CAVEAT};
    use crate::tokens::MIN_SAMPLE_PER_ARM;

    const DAY: i64 = 86_400;

    fn obs(citation: Citation, used: Option<u64>, reask: ReAskObservation) -> StepObservation {
        StepObservation {
            project_id: "p1".into(),
            issue_id: "iss_1".into(),
            session: "acp:c1".into(),
            column: "Plan".into(),
            settled_at: 0,
            citation,
            wrote_dossier: false,
            reask,
            context_used: used,
            context_size: Some(200_000),
            fidelity: Fidelity::Full,
        }
    }

    fn scanned(memory_answered: u32, human_asked: u32) -> ReAskObservation {
        ReAskObservation::Scanned(ReAskTally {
            memory_answered,
            human_asked,
        })
    }

    /// No dossiers, no transcripts, no cycles: four honest blanks.
    #[test]
    fn an_empty_project_reports_unknowns_rather_than_zeroes() {
        let value = empty("p1", DEFAULT_WINDOW_DAYS, 0);
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
        for line in value.lines() {
            assert!(!line.is_empty());
        }
        assert!(value.lines()[3].contains(TREND_CAVEAT));
    }

    #[test]
    fn a_constructed_window_folds_to_the_known_answer() {
        let mut all = [
            obs(Citation::CitedRead, Some(10_000), scanned(2, 0)),
            obs(
                Citation::CitedRead,
                Some(12_000),
                ReAskObservation::Scanned(ReAskTally::default()),
            ),
            obs(
                Citation::CitedMention,
                Some(14_000),
                ReAskObservation::Scanned(ReAskTally::default()),
            ),
            obs(
                Citation::NotCited,
                Some(30_000),
                ReAskObservation::Scanned(ReAskTally::default()),
            ),
            obs(
                Citation::NotCited,
                Some(32_000),
                ReAskObservation::Scanned(ReAskTally::default()),
            ),
            obs(
                Citation::NotCited,
                Some(34_000),
                ReAskObservation::Scanned(ReAskTally::default()),
            ),
            obs(Citation::Unknown, None, ReAskObservation::Unreadable),
        ];
        // The last one never had a readable transcript.
        all[6].fidelity = Fidelity::Absent;
        let refs: Vec<&StepObservation> = all.iter().collect();
        let dossier_tallies = [DatedTally {
            at: Some(10 * DAY),
            tally: ReAskTally {
                memory_answered: 1,
                human_asked: 1,
            },
        }];
        let value = compute(MemoryInputs {
            project_id: "p1",
            window_days: 30,
            since: 0,
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
                unknown_with_hit: 0,
                wrote_without_reading: 0,
            }
        );
        // 3 cited of 6 conclusive — the unreadable session is excluded, not
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

    /// A session that hit its scan budget can produce a hit but could never
    /// have produced a miss, so it stays out of both sides of the rate.
    #[test]
    fn a_hit_from_a_truncated_transcript_does_not_enter_the_rate() {
        let mut truncated = obs(Citation::CitedRead, Some(1), ReAskObservation::Unreadable);
        truncated.fidelity = Fidelity::Truncated;
        let full = obs(
            Citation::NotCited,
            Some(1),
            ReAskObservation::Scanned(ReAskTally::default()),
        );
        let all = [truncated, full];
        let refs: Vec<&StepObservation> = all.iter().collect();
        let citations = Citations::tally(refs.iter().map(|o| (o.citation, o.fidelity, false)));
        assert_eq!(citations.sessions, 2);
        assert_eq!(citations.unknown, 1);
        assert_eq!(citations.unknown_with_hit, 1);
        assert_eq!(citations.cited_read, 0);
        // 0 of 1, not 1 of 2.
        assert_eq!(citations.rate(), Some(0.0));
    }

    /// The window has to bound the dossier-derived halves too, or the label
    /// says "this month" over a figure covering all history.
    #[test]
    fn the_window_bounds_the_dossier_derived_halves() {
        let since = 100 * DAY;
        let dossier_tallies = [
            DatedTally {
                at: Some(10 * DAY), // older than the window
                tally: ReAskTally {
                    memory_answered: 9,
                    human_asked: 9,
                },
            },
            DatedTally {
                at: None, // undated: excluded and counted, never assumed recent
                tally: ReAskTally {
                    memory_answered: 5,
                    human_asked: 5,
                },
            },
            DatedTally {
                at: Some(120 * DAY),
                tally: ReAskTally {
                    memory_answered: 3,
                    human_asked: 1,
                },
            },
        ];
        let value = compute(MemoryInputs {
            project_id: "p1",
            window_days: 90,
            since,
            observations: &[],
            dossier_tallies: &dossier_tallies,
            cycles: vec![
                FeatureCycle {
                    created: 0,
                    landed: 5 * DAY, // landed before the window
                },
                FeatureCycle {
                    created: 110 * DAY,
                    landed: 111 * DAY,
                },
            ],
            dossiers_scanned: 3,
            clipped: false,
        });

        // Only the in-window section counted: 1 human of 4.
        assert_eq!(value.re_ask.rate(), Some(0.25));
        assert_eq!(value.sections_undated, 1);
        // One landing in the window is a point, not a two-feature trend.
        assert_eq!(
            value.time_to_land.read().0,
            TimeToLandKind::SinglePoint { hours: 24.0 }
        );
        assert_eq!(value.cycles_outside_window, 1);
        assert_eq!(value.window_since, since);
    }

    #[test]
    fn an_all_unknown_window_has_no_citation_rate() {
        let mut one = obs(Citation::Unknown, None, ReAskObservation::Unreadable);
        one.fidelity = Fidelity::Absent;
        let all = vec![one; 4];
        let refs: Vec<&StepObservation> = all.iter().collect();
        let value = compute(MemoryInputs {
            project_id: "p1",
            window_days: 90,
            since: 0,
            observations: &refs,
            dossier_tallies: &[],
            cycles: Vec::new(),
            dossiers_scanned: 0,
            clipped: false,
        });
        assert_eq!(value.citations.unknown, 4);
        assert_eq!(value.citations.rate(), None);
        assert!(value.citations.label().contains("unknown"));
        assert_eq!(value.re_ask.rate(), None);
    }

    #[test]
    fn the_payload_always_carries_the_time_to_land_caveat() {
        let json = serde_json::to_string(&empty("p1", 90, 0)).unwrap();
        assert!(json.contains("Trend, not attribution"), "{json}");
    }
}
