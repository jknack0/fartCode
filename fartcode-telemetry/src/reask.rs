//! Signal 2 — **re-ask rate**: clarifications the agent answered out of
//! project memory, versus the ones it had to put to the human again.
//!
//! ADR-0038's Consequences paragraph is explicit that this one cannot be
//! inferred: it "requires the grill steps to tag questions as
//! memory-answered vs. human-asked (a step-prompt convention, versioned
//! with the skill)". #73 therefore adds the convention as well as the
//! reader — see `fartcode_core::skills`, where both halves move together
//! under `FEATURE_LOG_VERSION`.
//!
//! # The tag
//!
//! ```text
//! - [fartcode:asked=memory] Which auth provider? — answered from docs/features/oauth-login.md
//! - [fartcode:asked=human] Should the token TTL be configurable?
//! ```
//!
//! Chosen for two properties:
//!
//! - **Cheap to emit.** One bracketed token at the head of a bullet the
//!   agent was already writing. No JSON, no block, no closing tag; a step
//!   that asks nothing writes nothing.
//! - **Unambiguous to parse.** `fartcode:` is this repo's established
//!   marker namespace (`<!-- fartcode:timeline -->`,
//!   `<!-- fartcode:feature-log-skill v1 -->`), so the token cannot collide
//!   with prose, and `rg '\[fartcode:asked='` finds every one. Two fixed
//!   literals, no regex, no grammar to drift.
//!
//! It is read from two places, because the two have opposite lifespans: the
//! **session transcript** (rich, but destroyed when the session ends — see
//! [`crate::observation`]) and the agent's **dossier section** (thinner,
//! but committed to the branch and still there a year later).
//!
//! # What happens when nobody emits it — and why it is not zero
//!
//! Most sessions will not carry a tag, at least at first: an older seeded
//! scaffold, an agent CLI that never saw the convention, a provider that
//! ignores the instruction, or — perfectly legitimately — a step that had
//! no clarification to record. None of those are "0% re-asked", and none
//! are "100% re-asked". They are *no observation*, and
//! [`ReAskRate::Unknown`] is what the type returns, with the number of
//! steps it looked at so the dashboard can say "not enough steps have
//! adopted the convention yet" instead of drawing a flat line at zero.
//!
//! The residual dishonesty, stated rather than hidden: a tagged step tells
//! us about the questions it *chose to tag*. An agent that quietly asks the
//! human without tagging looks like a step with no clarifications at all.
//! The signal is a floor on re-asking, never a census.

use serde::{Deserialize, Serialize};

use crate::citations::MAX_SCAN_BYTES;
use crate::observation::TranscriptView;

/// The tag for a clarification the agent resolved from project memory.
pub const TAG_MEMORY: &str = "[fartcode:asked=memory]";

/// The tag for a clarification the agent had to put to the human.
pub const TAG_HUMAN: &str = "[fartcode:asked=human]";

/// The shared prefix, so a caller can cheaply test whether any tagging
/// happened at all (and so the skill can document one grep).
pub const TAG_PREFIX: &str = "[fartcode:asked=";

/// Tag counts from one step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReAskTally {
    pub memory_answered: u32,
    pub human_asked: u32,
}

impl ReAskTally {
    pub fn total(&self) -> u32 {
        self.memory_answered + self.human_asked
    }

    /// True when this step said nothing either way — the case that must not
    /// be read as a zero.
    pub fn is_silent(&self) -> bool {
        self.total() == 0
    }

    pub fn merge(&mut self, other: ReAskTally) {
        self.memory_answered += other.memory_answered;
        self.human_asked += other.human_asked;
    }
}

/// Counts tags in arbitrary text — used for the durable half, the agent's
/// committed dossier section.
pub fn tally_text(text: &str) -> ReAskTally {
    ReAskTally {
        memory_answered: count(text, TAG_MEMORY),
        human_asked: count(text, TAG_HUMAN),
    }
}

/// Counts tags across a session transcript, skipping the injected prompt.
///
/// The skip is not an optimization. The seeded prompt *shows both tags* to
/// teach the format, so a naive whole-transcript scan would score one of
/// each for every consented step — a 50% re-ask rate manufactured by the
/// instruction that asked for the measurement.
pub fn tally(view: &TranscriptView<'_>) -> ReAskTally {
    let mut out = ReAskTally::default();
    let mut budget = MAX_SCAN_BYTES;
    for segment in &view.segments {
        if !segment.source.scannable() || segment.text.len() > budget {
            continue;
        }
        budget -= segment.text.len();
        out.merge(tally_text(segment.text));
    }
    out
}

fn count(haystack: &str, needle: &str) -> u32 {
    // `matches` is non-overlapping, which is right: the tags cannot nest.
    u32::try_from(haystack.matches(needle).count()).unwrap_or(u32::MAX)
}

/// The aggregate. Two variants, and the difference between them is the
/// point of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReAskRate {
    /// Not one of the `steps_scanned` steps emitted a tag. There is no
    /// rate — not zero, not a hundred percent. The dashboard should say
    /// how many steps it looked at and stop.
    Unknown { steps_scanned: u32 },
    /// At least one tag was seen.
    Observed {
        memory_answered: u32,
        human_asked: u32,
        /// How many steps contributed at least one tag, so a rate built
        /// from a single chatty step is visibly that.
        steps_tagged: u32,
        steps_scanned: u32,
    },
}

impl ReAskRate {
    /// Folds per-step tallies. A step with a silent tally is counted as
    /// scanned but contributes nothing.
    pub fn from_tallies<'a>(tallies: impl IntoIterator<Item = &'a ReAskTally>) -> Self {
        let mut sum = ReAskTally::default();
        let mut steps_scanned = 0u32;
        let mut steps_tagged = 0u32;
        for tally in tallies {
            steps_scanned += 1;
            if !tally.is_silent() {
                steps_tagged += 1;
                sum.merge(*tally);
            }
        }
        if sum.is_silent() {
            return ReAskRate::Unknown { steps_scanned };
        }
        ReAskRate::Observed {
            memory_answered: sum.memory_answered,
            human_asked: sum.human_asked,
            steps_tagged,
            steps_scanned,
        }
    }

    /// The fraction of tagged clarifications that went back to the human —
    /// 0.0 means memory answered all of them. `None` when there is no
    /// observation, which is a different thing from 0.0 and stays a
    /// different thing all the way to the caller.
    pub fn rate(&self) -> Option<f64> {
        match self {
            ReAskRate::Unknown { .. } => None,
            ReAskRate::Observed {
                memory_answered,
                human_asked,
                ..
            } => {
                let total = f64::from(*memory_answered + *human_asked);
                (total > 0.0).then(|| f64::from(*human_asked) / total)
            }
        }
    }

    /// One line for the dashboard, carrying the uncertainty in the words as
    /// well as the type.
    pub fn label(&self) -> String {
        match self {
            ReAskRate::Unknown { steps_scanned } => format!(
                "unknown — none of the {steps_scanned} step(s) in this window tagged a \
                 clarification (the convention may not have reached this project's agents yet)"
            ),
            ReAskRate::Observed {
                memory_answered,
                human_asked,
                steps_tagged,
                ..
            } => {
                let rate = self.rate().unwrap_or(0.0) * 100.0;
                format!(
                    "{rate:.0}% re-asked — {human_asked} put back to you, {memory_answered} \
                     answered from memory, across {steps_tagged} tagged step(s)"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{Fidelity, Segment, SegmentSource};

    #[test]
    fn tagged_text_parses() {
        let section = "\
## Grill — 2026-08-09

- [fartcode:asked=memory] Which auth provider? — answered from docs/features/oauth-login.md
- [fartcode:asked=memory] Token TTL? — the earlier dossier settled it
- [fartcode:asked=human] Should refresh tokens rotate?
";
        let tally = tally_text(section);
        assert_eq!(tally.memory_answered, 2);
        assert_eq!(tally.human_asked, 1);
        assert_eq!(tally.total(), 3);
        assert!(!tally.is_silent());
    }

    /// The degenerate case the ticket calls out by name.
    #[test]
    fn untagged_steps_degrade_to_unknown_not_zero_and_not_a_hundred() {
        let silent = [ReAskTally::default(); 3];
        let rate = ReAskRate::from_tallies(silent.iter());
        assert_eq!(rate, ReAskRate::Unknown { steps_scanned: 3 });
        assert_eq!(rate.rate(), None);
        assert!(rate.label().contains("unknown"));
        // Specifically NOT these:
        assert_ne!(rate.rate(), Some(0.0));
        assert_ne!(rate.rate(), Some(1.0));
    }

    #[test]
    fn no_steps_at_all_is_also_unknown() {
        let rate = ReAskRate::from_tallies(std::iter::empty());
        assert_eq!(rate, ReAskRate::Unknown { steps_scanned: 0 });
        assert_eq!(rate.rate(), None);
    }

    #[test]
    fn one_tagged_step_among_silent_ones_still_yields_a_rate() {
        let tallies = [
            ReAskTally::default(),
            ReAskTally {
                memory_answered: 3,
                human_asked: 1,
            },
            ReAskTally::default(),
        ];
        let rate = ReAskRate::from_tallies(tallies.iter());
        assert_eq!(
            rate,
            ReAskRate::Observed {
                memory_answered: 3,
                human_asked: 1,
                steps_tagged: 1,
                steps_scanned: 3,
            }
        );
        assert_eq!(rate.rate(), Some(0.25));
        assert!(rate.label().contains("25%"));
        // The sample size is visible, so "25%" cannot be read as a trend.
        assert!(rate.label().contains("1 tagged step"));
    }

    #[test]
    fn all_answered_from_memory_is_a_real_zero() {
        let tallies = [ReAskTally {
            memory_answered: 4,
            human_asked: 0,
        }];
        let rate = ReAskRate::from_tallies(tallies.iter());
        assert_eq!(rate.rate(), Some(0.0));
    }

    /// The prompt teaches the format by showing both tags. Scanning it
    /// would manufacture a 50% rate for every consented step.
    #[test]
    fn the_injected_prompt_examples_are_not_counted() {
        let prompt = "\
Tag each clarification:
- [fartcode:asked=memory] <question> — <where the answer came from>
- [fartcode:asked=human] <question>
";
        let view = TranscriptView {
            segments: vec![Segment::new(SegmentSource::InjectedPrompt, prompt)],
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Full,
        };
        assert_eq!(tally(&view), ReAskTally::default());
        assert!(tally(&view).is_silent());
    }

    #[test]
    fn transcript_tags_outside_the_prompt_are_counted() {
        let prompt = "- [fartcode:asked=memory] <question>\n- [fartcode:asked=human] <question>";
        let said = "- [fartcode:asked=human] Which region should this deploy to?";
        let view = TranscriptView {
            segments: vec![
                Segment::new(SegmentSource::InjectedPrompt, prompt),
                Segment::new(SegmentSource::AgentProse, said),
            ],
            context_used: None,
            context_size: None,
            fidelity: Fidelity::Full,
        };
        assert_eq!(
            tally(&view),
            ReAskTally {
                memory_answered: 0,
                human_asked: 1,
            }
        );
    }

    #[test]
    fn untagged_prose_is_silent_rather_than_guessed_at() {
        assert!(tally_text("I asked the user which provider to use.").is_silent());
        assert!(tally_text("").is_silent());
        // A near-miss is not a tag.
        assert!(tally_text("[asked=human] nope").is_silent());
        assert!(tally_text("[fartcode:asked=maybe] nope").is_silent());
    }
}
