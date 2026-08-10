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
//! # The teaching text must not parse as a tag
//!
//! The prompt and the skill file teach this format by **showing** it, in
//! the exact literals this module matches — that is the property that keeps
//! writer and reader from drifting. It is also a trap: the instruction is
//! then indistinguishable from an agent following it, and every transcript
//! that contains the prompt, every `cat SKILL.md`, and every agent that
//! quotes the convention back would score one tag of each kind.
//!
//! So the parser requires a **filled-in** tag: the first non-space
//! character after the literal must not be `<`. Both taught examples use
//! `<question>` placeholders, so the instruction is inert wherever it
//! appears, while a real clarification — which never begins with a `<` —
//! counts. [`is_placeholder`] is that rule, and
//! `fartcode_core::skills` has a test proving its own teaching text parses
//! to nothing.
//!
//! Tags are also required to open a line (optionally behind a `- ` bullet
//! or fence indentation), so prose that merely names the convention is not
//! a clarification either.
//!
//! # Where it is read from, and where it deliberately is not
//!
//! Two sources, with opposite lifespans: the **session transcript** (rich,
//! but destroyed when the session ends — see [`crate::observation`]) and
//! the agent's **dossier section** (thinner, but committed to the branch
//! and still there a year later).
//!
//! [`tally`] refuses to read a transcript that is not
//! [`Fidelity::Full`], and that refusal lives here rather than in the app
//! layer on purpose — it mirrors [`crate::citations::CitationScan::verdict`],
//! which already declines to rule on a truncated view. The case it closes
//! is a flattened PTY scrollback: the terminal echoes the bracket-pasted
//! prompt, so a scan of it finds the two taught literals and reports a
//! fabricated 50% re-ask rate for a project where no agent ever emitted a
//! tag. A rule that the caller has to remember is a rule that the next
//! caller will not.
//!
//! # What happens when nobody emits it — and why it is not zero
//!
//! Most sessions will not carry a tag, at least at first: an older seeded
//! scaffold, an agent CLI that never saw the convention, a provider that
//! ignores the instruction, or — perfectly legitimately — a step that had
//! no clarification to record. None of those are "0% re-asked", and none
//! are "100% re-asked". They are *no observation*, and
//! [`ReAskRate::Unknown`] is what the type returns, with the number of
//! steps it looked at and the number it could not read, so the dashboard
//! can tell "nobody has adopted this yet" from "we could not see".
//!
//! The residual dishonesty, stated rather than hidden: a tagged step tells
//! us about the questions it *chose to tag*. An agent that quietly asks the
//! human without tagging looks like a step with no clarifications at all.
//! The signal is a floor on re-asking, never a census.

use serde::{Deserialize, Serialize};

use crate::citations::MAX_SCAN_BYTES;
use crate::observation::{Fidelity, ReAskObservation, TranscriptView};

/// The tag for a clarification the agent resolved from project memory.
pub const TAG_MEMORY: &str = "[fartcode:asked=memory]";

/// The tag for a clarification the agent had to put to the human.
pub const TAG_HUMAN: &str = "[fartcode:asked=human]";

/// The shared prefix, so a caller can cheaply test whether any tagging
/// happened at all (and so the skill can document one grep).
pub const TAG_PREFIX: &str = "[fartcode:asked=";

/// The character that opens a placeholder in the taught examples. A tag
/// followed by one of these is documentation, not a clarification.
const PLACEHOLDER_OPEN: char = '<';

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

/// Whether the text directly after a tag is a placeholder rather than a
/// real question — i.e. whether this occurrence is the convention being
/// *taught* rather than *used*.
///
/// `rest` is everything after the tag literal. An empty remainder counts as
/// a placeholder too: a bare tag records nothing.
pub fn is_placeholder(rest: &str) -> bool {
    match rest.trim_start().chars().next() {
        None => true,
        Some(c) => c == PLACEHOLDER_OPEN,
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

/// Reads a session transcript for tags.
///
/// Returns [`ReAskObservation::Unreadable`] for anything short of
/// [`Fidelity::Full`] — see the module docs. Non-scannable segments (the
/// injected prompt, a flattened scrollback) contribute nothing even within
/// a `Full` view.
pub fn tally(view: &TranscriptView<'_>) -> ReAskObservation {
    if view.fidelity != Fidelity::Full {
        return ReAskObservation::Unreadable;
    }
    let mut out = ReAskTally::default();
    let mut budget = MAX_SCAN_BYTES;
    for segment in &view.segments {
        if !segment.source.scannable() {
            continue;
        }
        if segment.text.len() > budget {
            // Ran out of budget mid-transcript: what is left is unread, so
            // the honest answer for the whole session is "could not read".
            return ReAskObservation::Unreadable;
        }
        budget -= segment.text.len();
        out.merge(tally_text(segment.text));
    }
    ReAskObservation::Scanned(out)
}

/// Counts line-opening, filled-in occurrences of `needle`.
fn count(haystack: &str, needle: &str) -> u32 {
    let mut count = 0u32;
    for line in haystack.lines() {
        // A tag opens its line, optionally behind a bullet or indentation,
        // so prose that merely names the convention is not a clarification.
        let body = line.trim_start();
        let body = body.strip_prefix("- ").unwrap_or(body).trim_start();
        let Some(rest) = body.strip_prefix(needle) else {
            continue;
        };
        if !is_placeholder(rest) {
            count = count.saturating_add(1);
        }
    }
    count
}

/// The aggregate. Two variants, and the difference between them is the
/// point of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReAskRate {
    /// Not one readable step emitted a tag. There is no rate — not zero,
    /// not a hundred percent.
    Unknown {
        /// Steps whose transcript (or dossier section) was scanned.
        steps_scanned: u32,
        /// Steps that could not be scanned at all. A window that is mostly
        /// this is a window with nothing to say, not a well-behaved one.
        steps_unreadable: u32,
    },
    /// At least one tag was seen.
    Observed {
        memory_answered: u32,
        human_asked: u32,
        /// How many steps contributed at least one tag, so a rate built
        /// from a single chatty step is visibly that.
        steps_tagged: u32,
        steps_scanned: u32,
        steps_unreadable: u32,
    },
}

impl ReAskRate {
    /// Folds per-step observations. An [`ReAskObservation::Unreadable`]
    /// step counts toward `steps_unreadable` and never toward
    /// `steps_scanned` — the asymmetry citations already draw.
    pub fn from_observations(observations: impl IntoIterator<Item = ReAskObservation>) -> Self {
        let mut sum = ReAskTally::default();
        let mut steps_scanned = 0u32;
        let mut steps_tagged = 0u32;
        let mut steps_unreadable = 0u32;
        for observation in observations {
            match observation {
                ReAskObservation::Unreadable => steps_unreadable += 1,
                ReAskObservation::Scanned(tally) => {
                    steps_scanned += 1;
                    if !tally.is_silent() {
                        steps_tagged += 1;
                        sum.merge(tally);
                    }
                }
            }
        }
        if sum.is_silent() {
            return ReAskRate::Unknown {
                steps_scanned,
                steps_unreadable,
            };
        }
        ReAskRate::Observed {
            memory_answered: sum.memory_answered,
            human_asked: sum.human_asked,
            steps_tagged,
            steps_scanned,
            steps_unreadable,
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
            ReAskRate::Unknown {
                steps_scanned,
                steps_unreadable,
            } => {
                let mut out = format!(
                    "unknown — none of the {steps_scanned} readable step(s) in this window \
                     tagged a clarification (the convention may not have reached this \
                     project's agents yet)"
                );
                if *steps_unreadable > 0 {
                    out.push_str(&format!(
                        "; a further {steps_unreadable} step(s) could not be read at all"
                    ));
                }
                out
            }
            ReAskRate::Observed {
                memory_answered,
                human_asked,
                steps_tagged,
                steps_unreadable,
                ..
            } => {
                let rate = self.rate().unwrap_or(0.0) * 100.0;
                let mut out = format!(
                    "{rate:.0}% re-asked — {human_asked} put back to you, {memory_answered} \
                     answered from memory, across {steps_tagged} tagged step(s)"
                );
                if *steps_unreadable > 0 {
                    out.push_str(&format!(
                        "; {steps_unreadable} step(s) could not be read and are excluded"
                    ));
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{Segment, SegmentSource};

    fn view(fidelity: Fidelity, segments: Vec<Segment<'_>>) -> TranscriptView<'_> {
        TranscriptView {
            segments,
            context_used: None,
            context_size: None,
            fidelity,
        }
    }

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

    /// The teaching text is written in the same literals the parser
    /// matches. It must still parse to nothing, wherever it turns up — the
    /// prompt, a `cat SKILL.md`, an agent quoting the convention back.
    #[test]
    fn the_taught_placeholder_examples_parse_to_nothing() {
        let taught = "\
```markdown
[fartcode:asked=memory] <question> — <where in project memory you found it>
[fartcode:asked=human] <question you had to put to the human>
```
";
        assert!(tally_text(taught).is_silent());
        // Bulleted, as the skill file writes them.
        assert!(tally_text("- [fartcode:asked=human] <question>").is_silent());
        // A bare tag records nothing either.
        assert!(tally_text("[fartcode:asked=memory]").is_silent());
        assert!(is_placeholder(" <question>"));
        assert!(is_placeholder(""));
        assert!(!is_placeholder(" Which provider?"));
    }

    #[test]
    fn a_tag_named_in_prose_is_not_a_clarification() {
        assert!(
            tally_text("we use the [fartcode:asked=memory] Which provider? convention").is_silent()
        );
    }

    /// The degenerate case the ticket calls out by name.
    #[test]
    fn untagged_steps_degrade_to_unknown_not_zero_and_not_a_hundred() {
        let silent = [ReAskObservation::Scanned(ReAskTally::default()); 3];
        let rate = ReAskRate::from_observations(silent);
        assert_eq!(
            rate,
            ReAskRate::Unknown {
                steps_scanned: 3,
                steps_unreadable: 0
            }
        );
        assert_eq!(rate.rate(), None);
        assert!(rate.label().contains("unknown"));
        // Specifically NOT these:
        assert_ne!(rate.rate(), Some(0.0));
        assert_ne!(rate.rate(), Some(1.0));
    }

    /// Unreadable is its own axis: three steps nobody could look at is not
    /// three steps that tagged nothing.
    #[test]
    fn unreadable_steps_are_reported_separately_from_silent_ones() {
        let rate = ReAskRate::from_observations([
            ReAskObservation::Unreadable,
            ReAskObservation::Unreadable,
            ReAskObservation::Scanned(ReAskTally::default()),
        ]);
        assert_eq!(
            rate,
            ReAskRate::Unknown {
                steps_scanned: 1,
                steps_unreadable: 2
            }
        );
        assert!(
            rate.label().contains("could not be read"),
            "{}",
            rate.label()
        );
    }

    #[test]
    fn no_steps_at_all_is_also_unknown() {
        let rate = ReAskRate::from_observations(std::iter::empty());
        assert_eq!(
            rate,
            ReAskRate::Unknown {
                steps_scanned: 0,
                steps_unreadable: 0
            }
        );
        assert_eq!(rate.rate(), None);
    }

    #[test]
    fn one_tagged_step_among_silent_ones_still_yields_a_rate() {
        let rate = ReAskRate::from_observations([
            ReAskObservation::Scanned(ReAskTally::default()),
            ReAskObservation::Scanned(ReAskTally {
                memory_answered: 3,
                human_asked: 1,
            }),
            ReAskObservation::Unreadable,
        ]);
        assert_eq!(
            rate,
            ReAskRate::Observed {
                memory_answered: 3,
                human_asked: 1,
                steps_tagged: 1,
                steps_scanned: 2,
                steps_unreadable: 1,
            }
        );
        assert_eq!(rate.rate(), Some(0.25));
        assert!(rate.label().contains("25%"));
        // The sample size is visible, so "25%" cannot be read as a trend.
        assert!(rate.label().contains("1 tagged step"));
    }

    #[test]
    fn all_answered_from_memory_is_a_real_zero() {
        let rate = ReAskRate::from_observations([ReAskObservation::Scanned(ReAskTally {
            memory_answered: 4,
            human_asked: 0,
        })]);
        assert_eq!(rate.rate(), Some(0.0));
    }

    /// The prompt teaches the format by showing both tags. Scanning it
    /// would manufacture a 50% rate for every consented step — twice over,
    /// since the placeholder rule and the provenance rule each close it.
    #[test]
    fn the_injected_prompt_examples_are_not_counted() {
        let prompt = "\
Tag each clarification:
- [fartcode:asked=memory] <question> — <where the answer came from>
- [fartcode:asked=human] <question>
";
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::InjectedPrompt, prompt)],
        );
        assert_eq!(tally(&v), ReAskObservation::Scanned(ReAskTally::default()));
        // ...and even with its provenance stripped, the placeholders save it.
        let stripped = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::AgentProse, prompt)],
        );
        assert_eq!(
            tally(&stripped),
            ReAskObservation::Scanned(ReAskTally::default())
        );
    }

    /// The headline PTY bug: a terminal echoes the bracket-pasted prompt,
    /// so scanning a flattened scrollback invents a 50% re-ask rate. Two
    /// independent guards now refuse it.
    #[test]
    fn a_flattened_scrollback_is_unreadable_not_a_fabricated_rate() {
        let echoed = "\
$ claude
> ## Feature log
> Tag each clarification:
> [fartcode:asked=memory] <question> — <where the answer came from>
> [fartcode:asked=human] <question>
working...
";
        let v = view(
            Fidelity::Truncated,
            vec![Segment::new(SegmentSource::Unstructured, echoed)],
        );
        assert_eq!(tally(&v), ReAskObservation::Unreadable);
        // A whole project of these is Unknown, never 50%.
        let rate = ReAskRate::from_observations([tally(&v), tally(&v), tally(&v)]);
        assert_eq!(
            rate,
            ReAskRate::Unknown {
                steps_scanned: 0,
                steps_unreadable: 3
            }
        );
        assert_eq!(rate.rate(), None);
    }

    #[test]
    fn transcript_tags_outside_the_prompt_are_counted() {
        let prompt = "- [fartcode:asked=memory] <question>\n- [fartcode:asked=human] <question>";
        let said = "- [fartcode:asked=human] Which region should this deploy to?";
        let v = view(
            Fidelity::Full,
            vec![
                Segment::new(SegmentSource::InjectedPrompt, prompt),
                Segment::new(SegmentSource::AgentProse, said),
            ],
        );
        assert_eq!(
            tally(&v),
            ReAskObservation::Scanned(ReAskTally {
                memory_answered: 0,
                human_asked: 1,
            })
        );
    }

    /// The fidelity gate is structural: even a perfectly tagged transcript
    /// is refused when only part of it was in hand, because the part we did
    /// not see is where the other tags would be.
    #[test]
    fn a_truncated_transcript_is_never_tallied() {
        let v = view(
            Fidelity::Truncated,
            vec![Segment::new(
                SegmentSource::AgentProse,
                "- [fartcode:asked=human] Which region?",
            )],
        );
        assert_eq!(tally(&v), ReAskObservation::Unreadable);
        assert_eq!(
            tally(&TranscriptView::absent()),
            ReAskObservation::Unreadable
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
