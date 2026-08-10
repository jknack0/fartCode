//! Signal 1 — **memory citations**: step sessions that referenced dossier
//! content.
//!
//! ADR-0038's Consequences paragraph names the mechanism: "citations
//! require detecting dossier references in transcripts (path-mention
//! heuristic first)". This is that heuristic, with three corrections the
//! bare version needs before its output means anything.
//!
//! **Correction 1 — the prompt is not a citation.**
//! `skills::append_instruction` puts the dossier path into every seeded
//! step prompt. A path-mention scan over the whole transcript therefore
//! reports 100% for every consented project, forever, before an agent has
//! read a byte. Segments carry their provenance
//! ([`crate::observation::SegmentSource`]) and the injected prompt is
//! skipped.
//!
//! **Correction 2 — writing is not reading.** The same prompt asks every
//! step to *append* a section, so the dossier path also shows up in the
//! agent's own edit. Write-ish tool calls are counted separately and never
//! score a citation; a read/grep tool call that resolves the dossier is the
//! strong tier, and prose naming it is the weak one. Shell commands are
//! routed to one of those tiers by [`classify_shell`] rather than sitting
//! flat at the weak one — otherwise `git add docs/features/x.md`, the
//! staging step the prompt's own instruction leads to, scores as a
//! citation, while `rg docs/features/`, which is the read the seeded skill
//! actually teaches, scores nothing.
//!
//! **Correction 3 — text with no provenance cannot be corrected at all.**
//! Corrections 1 and 2 both key off
//! [`crate::observation::SegmentSource`], so they hold only while
//! provenance survives. A flattened terminal scrollback has none *and*
//! echoes the injected prompt, so it is not scanned — see
//! [`crate::observation::SegmentSource::Unstructured`], which is where that
//! decision and its cost are argued.
//!
//! **Rejected: matching section headings.** The ticket floats headings as a
//! second needle. They fail correction 2 badly — an agent appending
//! `## Plan — 2026-08-09` emits that exact string, so heading matches score
//! authorship as citation at a high rate and cannot be told apart from a
//! genuine quote. Path mentions scoped by segment provenance carry the same
//! information without the bias, so headings are not scanned.
//!
//! **Honest error modes.**
//! - *False positive:* a vendored subtree with a same-named dossier at the
//!   same repo-relative path, opened for an unrelated reason. Rare; not
//!   detectable from text alone.
//! - *False negative:* an agent that read the dossier through a tool whose
//!   ACP summary omits the path, or whose mention scrolled out of a PTY
//!   ring. This is why a miss on a truncated transcript is
//!   [`Citation::Unknown`], not [`Citation::NotCited`].
//! - *Not measured:* whether the content read actually changed what the
//!   agent did. Nothing in a transcript can answer that.

use serde::{Deserialize, Serialize};

use crate::observation::{Fidelity, TranscriptView};

/// What a session would have had to mention to count as citing.
#[derive(Debug, Clone, Copy)]
pub struct CitationTargets<'a> {
    /// Repo-relative dossier paths belonging to the card this session ran
    /// for, e.g. `docs/features/oauth-login.md`. The **full path**, never
    /// the slug: `src/oauth-login/handler.rs` is a source file, not a
    /// reference to the feature's memory.
    pub paths: &'a [String],
    /// The dossier directory, e.g. `docs/features`. Matched only inside
    /// read-ish segments, and only as the directory *itself* (see
    /// [`mentions_dir`]), because the seeded skill tells agents to grep the
    /// whole directory (`rg -n "rate limit" docs/features/`) and that is a
    /// consumption of memory even though no single file is named.
    pub dir: &'a str,
}

/// The verdict for one session. Ordered from strongest to least
/// informative; `Unknown` is a first-class answer, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Citation {
    /// A read or search tool call resolved the dossier (or its directory).
    /// The strongest evidence available that memory was consumed.
    CitedRead,
    /// The dossier was named in agent prose or a shell line, but no read
    /// tool call resolved it. Real, weaker: the agent may have been
    /// talking about the file rather than reading it.
    CitedMention,
    /// The whole transcript was in hand and contains no reference.
    NotCited,
    /// Nothing can be said. Either no transcript was available, or only a
    /// truncated tail was and it happened to contain no hit — silence from
    /// a 64 KiB window over an hour-long session is not a "no".
    Unknown,
}

impl Citation {
    /// Whether this counts toward the numerator of the citation rate.
    pub fn is_cited(&self) -> bool {
        matches!(self, Citation::CitedRead | Citation::CitedMention)
    }

    /// Whether this session can appear in the rate at all. `Unknown`
    /// sessions are reported alongside it, never folded into it.
    pub fn is_conclusive(&self) -> bool {
        !matches!(self, Citation::Unknown)
    }
}

/// Per-segment-tier hit counts for one session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CitationScan {
    /// Read/search segments that resolved the dossier or its directory.
    pub read_hits: u32,
    /// Prose or shell segments naming the dossier.
    pub prose_hits: u32,
    /// Write segments naming it. Recorded so the dashboard can say "the
    /// agent wrote its section" — never a citation.
    pub write_hits: u32,
}

impl CitationScan {
    /// The verdict, given how much of the transcript was readable.
    pub fn verdict(&self, fidelity: Fidelity) -> Citation {
        if self.read_hits > 0 {
            return Citation::CitedRead;
        }
        if self.prose_hits > 0 {
            return Citation::CitedMention;
        }
        match fidelity {
            // A complete transcript with no mention is a real "no".
            Fidelity::Full => Citation::NotCited,
            // A tail, or nothing at all, cannot rule a mention out.
            Fidelity::Truncated | Fidelity::Absent => Citation::Unknown,
        }
    }
}

/// Bound on how much text one session's scan will read.
///
/// **Why bounded.** A long agent session can hold megabytes of tool output,
/// and this scan runs on the settle path where it must never be felt. 2 MiB
/// is far more than any real transcript's *prose and paths* (the app layer
/// feeds summaries and paths, not file bodies) while still capping the
/// worst case at a few milliseconds of substring search. Overrun is not
/// silent: the scan stops and the caller reports the session at
/// [`Fidelity::Truncated`], so a miss degrades to `Unknown`.
pub const MAX_SCAN_BYTES: usize = 2 * 1024 * 1024;

/// Scans one session. Returns the tier counts and whether the byte budget
/// clipped the scan (in which case the caller must downgrade fidelity).
pub fn scan(view: &TranscriptView<'_>, targets: &CitationTargets<'_>) -> (CitationScan, bool) {
    let mut out = CitationScan::default();
    let mut budget = MAX_SCAN_BYTES;
    let mut clipped = false;

    for segment in &view.segments {
        if !segment.source.scannable() {
            continue;
        }
        if segment.text.len() > budget {
            clipped = true;
            break;
        }
        budget -= segment.text.len();

        let named = targets
            .paths
            .iter()
            .any(|path| mentions_path(segment.text, path));
        // A directory mention only means "consumed memory" in a read-ish
        // segment; in prose it is usually the agent quoting the convention
        // back, and in a write it is the append it was told to make.
        let grepped = segment.source.is_read()
            && !targets.dir.is_empty()
            && mentions_dir(segment.text, targets.dir);

        if !named && !grepped {
            continue;
        }
        if segment.source.is_read() {
            out.read_hits += 1;
        } else if segment.source.is_write() {
            out.write_hits += 1;
        } else {
            out.prose_hits += 1;
        }
    }

    (out, clipped)
}

/// Scans and rules in one call.
///
/// Returns the **folded fidelity** as well as the verdict, and the caller
/// is expected to persist it: a scan that ran out of budget over a `Full`
/// transcript is no longer `Full`, and if that fact is dropped the session
/// can still contribute a positive while never being able to contribute a
/// negative. A sample that can only add to the numerator is a sample that
/// can only flatter, which is the failure this whole module is shaped
/// against.
pub fn verdict(
    view: &TranscriptView<'_>,
    targets: &CitationTargets<'_>,
) -> (Citation, CitationScan, Fidelity) {
    let (scan_result, clipped) = scan(view, targets);
    let fidelity = match (clipped, view.fidelity) {
        (true, Fidelity::Full) => Fidelity::Truncated,
        (_, fidelity) => fidelity,
    };
    (scan_result.verdict(fidelity), scan_result, fidelity)
}

// ---------------------------------------------------------------------------
// Shell intent
// ---------------------------------------------------------------------------

/// What a shell command was going to do to the path it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIntent {
    /// `rg`, `cat`, `grep`… — consuming the file. The seeded skill teaches
    /// `rg -n "…" docs/features/`, so this is how a real citation most
    /// often looks on the shell.
    Read,
    /// `git add`, `mv`, `>`… — producing, staging or moving it. The seeded
    /// prompt tells every step to append a section and most agents then
    /// stage it, so this is the *expected background* on any consented
    /// project and must never reach the citation numerator.
    Write,
    /// Neither recognized. Scored at the weak prose tier rather than forced
    /// into one of the two.
    Ambiguous,
}

/// Verbs and operators that mean the command produced or moved a file.
///
/// Checked first and matched anywhere in the line: a pipeline that both
/// reads and writes has, on balance, written. The asymmetry is deliberate —
/// guessing wrong this way costs a missed citation, guessing wrong the
/// other way puts authorship in the numerator, and only one of those two
/// errors makes the number look better than the truth.
const WRITE_TOKENS: &[&str] = &[
    "git add ",
    "git commit",
    "git rm ",
    "git mv ",
    "git stash",
    "git restore",
    "git checkout",
    "git apply",
    "rm ",
    "mv ",
    "cp ",
    "tee ",
    "sed -i",
    "touch ",
    ">>",
    "> ",
];

/// Verbs that mean the command consumed a file.
const READ_TOKENS: &[&str] = &[
    "rg ", "grep ", "cat ", "bat ", "head ", "tail ", "less ", "more ", "sed -n", "awk ", "find ",
    "ls ", "git show", "git log", "git diff", "git grep", "wc ", "nl ",
];

/// Classifies a shell command line.
///
/// A deliberately small substring table rather than a grammar: commands
/// arrive as free text from any of 35 providers, so a parser would be for a
/// language nobody agreed on. Anything unrecognized stays
/// [`ShellIntent::Ambiguous`].
pub fn classify_shell(command: &str) -> ShellIntent {
    // Padded so a line that ENDS in the verb's argument still matches the
    // trailing space the tokens carry (which is what keeps `rm ` from
    // matching `--dry-run`).
    let padded = format!("{command} ");
    if WRITE_TOKENS.iter().any(|token| padded.contains(token)) {
        return ShellIntent::Write;
    }
    if READ_TOKENS.iter().any(|token| padded.contains(token)) {
        return ShellIntent::Read;
    }
    ShellIntent::Ambiguous
}

impl ShellIntent {
    /// The segment provenance a command of this intent should carry, so the
    /// app layer holds no verb knowledge of its own.
    pub fn source(&self) -> crate::observation::SegmentSource {
        use crate::observation::SegmentSource;
        match self {
            ShellIntent::Read => SegmentSource::ToolRead,
            ShellIntent::Write => SegmentSource::ToolWrite,
            ShellIntent::Ambiguous => SegmentSource::ToolExec,
        }
    }
}

// ---------------------------------------------------------------------------
// Path matching — the boundary rules are the whole heuristic
// ---------------------------------------------------------------------------

/// Characters that continue a path segment. A match flanked by one of these
/// is part of a *different* name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// The needle is a file: nothing may extend it.
    File,
    /// The needle is a directory: it may be followed by a trailing `/`,
    /// but **not by a name under it**. `rg docs/features/` is a search of
    /// the memory directory; `docs/features/oauth-login.md.bak` is one
    /// specific file, which has to match a target path on its own merits.
    Dir,
}

/// Whether `haystack` names the exact file `needle`.
///
/// Left boundary: a `/` is fine — transcripts print absolute paths
/// (`/Users/x/wt-oauth/docs/features/oauth-login.md`) and the leading
/// worktree root does not change which file it is. A name character is not:
/// `mydocs/features/oauth-login.md` is a different tree.
///
/// Right boundary: nothing may extend the name. `…/oauth-login.md.bak`,
/// `…/oauth-login.mdx` and `…/oauth-login.md/x` are all rejected, while a
/// sentence-final `…/oauth-login.md.` is accepted.
pub fn mentions_path(haystack: &str, needle: &str) -> bool {
    matches(haystack, needle, Shape::File)
}

/// Whether `haystack` names the memory directory `needle` **itself** — a
/// directory-scoped read, the `rg -n "rate limit" docs/features/` the
/// seeded skill teaches.
///
/// Not "anything under it": a specific file under `docs/features/` is a
/// specific file, and it counts only when it matches one of the card's
/// target paths. Otherwise every backup, draft and other feature's dossier
/// that happened to be opened would score a citation of *this* card's
/// memory.
pub fn mentions_dir(haystack: &str, needle: &str) -> bool {
    matches(haystack, needle, Shape::Dir)
}

fn matches(haystack: &str, needle: &str, shape: Shape) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        if left_ok(haystack, start) && right_ok(haystack, end, shape) {
            return true;
        }
        // Advance by one char, not one byte — `haystack[from..]` must stay
        // on a boundary.
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
        if from >= haystack.len() {
            break;
        }
    }
    false
}

fn left_ok(haystack: &str, start: usize) -> bool {
    match haystack[..start].chars().next_back() {
        None => true,
        // `./docs/features/x.md` is fine: the char before `docs` is `/`.
        // `a.docs/...` is not — that is somebody else's directory.
        Some(c) => !is_name_char(c) && c != '.',
    }
}

fn right_ok(haystack: &str, end: usize, shape: Shape) -> bool {
    let mut rest = haystack[end..].chars();
    match rest.next() {
        None => true,
        // A trailing slash ends a directory mention, unless a name follows.
        Some('/') => shape == Shape::Dir && !rest.next().is_some_and(is_name_char),
        Some(c) if is_name_char(c) => false,
        // `.md.bak` extends the name; `.md.` ending a sentence does not.
        Some('.') => !rest.next().is_some_and(|c| c.is_ascii_alphanumeric()),
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{Segment, SegmentSource};

    const DOSSIER: &str = "docs/features/oauth-login.md";

    fn view(fidelity: Fidelity, segments: Vec<Segment<'_>>) -> TranscriptView<'_> {
        TranscriptView {
            segments,
            context_used: None,
            context_size: None,
            fidelity,
        }
    }

    fn targets(paths: &[String]) -> CitationTargets<'_> {
        CitationTargets {
            paths,
            dir: "docs/features",
        }
    }

    #[test]
    fn a_read_tool_call_naming_the_dossier_is_a_strong_citation() {
        let paths = vec![DOSSIER.to_string()];
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolRead, DOSSIER)],
        );
        let (verdict_, scan_, _) = verdict(&v, &targets(&paths));
        assert_eq!(verdict_, Citation::CitedRead);
        assert_eq!(scan_.read_hits, 1);
    }

    #[test]
    fn an_absolute_path_in_a_worktree_still_counts() {
        let paths = vec![DOSSIER.to_string()];
        let text = "/Users/x/.fartCode/worktrees/oauth/docs/features/oauth-login.md";
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolRead, text)],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::CitedRead);
    }

    /// The heuristic's headline failure mode, closed: a source file that
    /// merely contains the feature slug is not a reference to its memory.
    #[test]
    fn an_unrelated_path_containing_the_slug_is_not_a_citation() {
        let paths = vec![DOSSIER.to_string()];
        for text in [
            "src/oauth-login/handler.rs",
            "tests/oauth-login.rs",
            "vendor/mydocs/features/oauth-login.md",
            "docs/features/oauth-login.md.bak",
            "docs/features/oauth-login.mdx",
            "docs/features/oauth-login-v2.md",
        ] {
            let v = view(
                Fidelity::Full,
                vec![Segment::new(SegmentSource::ToolRead, text)],
            );
            assert_eq!(
                verdict(&v, &targets(&paths)).0,
                Citation::NotCited,
                "{text} must not count"
            );
        }
    }

    #[test]
    fn a_sentence_final_period_does_not_break_the_match() {
        let paths = vec![DOSSIER.to_string()];
        let v = view(
            Fidelity::Full,
            vec![Segment::new(
                SegmentSource::AgentProse,
                "I read docs/features/oauth-login.md. It says to keep it append-only.",
            )],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::CitedMention);
    }

    /// The whole reason segments carry provenance: the seeded prompt
    /// contains the path, so scanning it would score every consented step.
    #[test]
    fn the_injected_prompt_alone_never_scores_a_citation() {
        let paths = vec![DOSSIER.to_string()];
        let prompt = "This feature keeps a decision log at `docs/features/oauth-login.md`.";
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::InjectedPrompt, prompt)],
        );
        let (verdict_, scan_, _) = verdict(&v, &targets(&paths));
        assert_eq!(verdict_, Citation::NotCited);
        assert_eq!(scan_, CitationScan::default());
    }

    /// Appending the section the prompt asked for is authorship, not
    /// reference.
    #[test]
    fn writing_the_dossier_is_not_citing_it() {
        let paths = vec![DOSSIER.to_string()];
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolWrite, DOSSIER)],
        );
        let (verdict_, scan_, _) = verdict(&v, &targets(&paths));
        assert_eq!(verdict_, Citation::NotCited);
        assert_eq!(scan_.write_hits, 1);
        assert_eq!(scan_.read_hits, 0);
    }

    /// The directory rule must not become "anything under `docs/features/`
    /// counts": another card's dossier is not this card's memory, and a
    /// stray `.bak` is not memory at all.
    #[test]
    fn a_read_of_a_different_file_under_the_directory_is_not_this_cards_citation() {
        let paths = vec![DOSSIER.to_string()];
        for text in [
            "docs/features/billing-retries.md",
            "docs/features/oauth-login.md.bak",
            "docs/features/draft.txt",
        ] {
            let v = view(
                Fidelity::Full,
                vec![Segment::new(SegmentSource::ToolRead, text)],
            );
            assert_eq!(
                verdict(&v, &targets(&paths)).0,
                Citation::NotCited,
                "{text} must not count"
            );
        }
    }

    #[test]
    fn grepping_the_directory_counts_only_from_a_read_segment() {
        let paths = vec![DOSSIER.to_string()];
        let read = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolRead, "docs/features/")],
        );
        assert_eq!(verdict(&read, &targets(&paths)).0, Citation::CitedRead);

        let prose = view(
            Fidelity::Full,
            vec![Segment::new(
                SegmentSource::AgentProse,
                "dossiers live in docs/features/",
            )],
        );
        assert_eq!(verdict(&prose, &targets(&paths)).0, Citation::NotCited);
    }

    /// The degenerate case that must not become a wrong number.
    #[test]
    fn silence_from_a_truncated_transcript_is_unknown_not_a_miss() {
        let paths = vec![DOSSIER.to_string()];
        let v = view(
            Fidelity::Truncated,
            vec![Segment::new(SegmentSource::Other, "nothing relevant here")],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::Unknown);

        let absent = TranscriptView::absent();
        assert_eq!(verdict(&absent, &targets(&paths)).0, Citation::Unknown);
    }

    /// ...but a hit in a tail is still a hit.
    #[test]
    fn a_hit_in_a_truncated_transcript_still_counts() {
        let paths = vec![DOSSIER.to_string()];
        let v = view(
            Fidelity::Truncated,
            vec![Segment::new(
                SegmentSource::Other,
                "cat docs/features/oauth-login.md",
            )],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::CitedMention);
    }

    #[test]
    fn a_scan_over_budget_downgrades_a_full_transcript_to_unknown() {
        let paths = vec![DOSSIER.to_string()];
        let huge = "x".repeat(MAX_SCAN_BYTES + 1);
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::AgentProse, &huge)],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::Unknown);
    }

    #[test]
    fn no_targets_means_nothing_to_cite() {
        let paths: Vec<String> = Vec::new();
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolRead, DOSSIER)],
        );
        let empty = CitationTargets {
            paths: &paths,
            dir: "",
        };
        assert_eq!(verdict(&v, &empty).0, Citation::NotCited);
    }

    #[test]
    fn matching_walks_past_a_rejected_hit_to_a_real_one() {
        let paths = vec![DOSSIER.to_string()];
        let text = "wrote mydocs/features/oauth-login.md then read docs/features/oauth-login.md";
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::ToolRead, text)],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::CitedRead);
    }

    /// A flattened scrollback echoes the injected prompt, so nothing in it
    /// can be attributed — including the dossier path the prompt itself
    /// names. Without this the mainstream dispatch path reports 100%.
    #[test]
    fn an_unprovenanced_scrollback_is_never_a_citation() {
        let paths = vec![DOSSIER.to_string()];
        let echoed =
            format!("$ claude\n> This feature keeps a decision log at `{DOSSIER}`.\nthinking...\n");
        let v = view(
            Fidelity::Truncated,
            vec![Segment::new(SegmentSource::Unstructured, &echoed)],
        );
        let (verdict_, scan_, fidelity) = verdict(&v, &targets(&paths));
        assert_eq!(verdict_, Citation::Unknown);
        assert_eq!(scan_, CitationScan::default());
        assert_eq!(fidelity, Fidelity::Truncated);
    }

    /// `git add docs/features/x.md` is the staging step the prompt's own
    /// instruction leads to. Scored flat at the prose tier it was a
    /// citation; classified, it is authorship.
    #[test]
    fn shell_intent_separates_the_mandated_write_from_a_real_read() {
        assert_eq!(
            classify_shell("git add docs/features/oauth-login.md"),
            ShellIntent::Write
        );
        assert_eq!(
            classify_shell("git commit -m 'dossier: plan'"),
            ShellIntent::Write
        );
        assert_eq!(
            classify_shell("echo hi > docs/features/oauth-login.md"),
            ShellIntent::Write
        );
        assert_eq!(
            classify_shell("rg -n 'rate limit' docs/features/"),
            ShellIntent::Read
        );
        assert_eq!(
            classify_shell("cat docs/features/oauth-login.md"),
            ShellIntent::Read
        );
        // A pipeline that both reads and writes counts as a write: the
        // error that direction costs a citation, the other costs the
        // metric its meaning.
        assert_eq!(
            classify_shell("cat a.md >> docs/features/oauth-login.md"),
            ShellIntent::Write
        );
        // Unrecognized stays unrecognized rather than being forced.
        assert_eq!(classify_shell("cargo test"), ShellIntent::Ambiguous);
        assert_eq!(classify_shell(""), ShellIntent::Ambiguous);
    }

    #[test]
    fn a_classified_shell_command_lands_in_the_right_tier() {
        let paths = vec![DOSSIER.to_string()];

        // The mandated staging is authorship, not a citation.
        let staged = format!("git add {DOSSIER}");
        let write = view(
            Fidelity::Full,
            vec![Segment::new(
                classify_shell(&staged).source(),
                staged.as_str(),
            )],
        );
        let (verdict_, scan_, _) = verdict(&write, &targets(&paths));
        assert_eq!(verdict_, Citation::NotCited);
        assert_eq!(scan_.write_hits, 1);

        // The grep the seeded skill teaches is a strong citation — it used
        // to score nothing at all, because the directory needle required a
        // read-ish segment and every shell line was scored as prose.
        let grep = "rg -n 'rate limit' docs/features/";
        let read = view(
            Fidelity::Full,
            vec![Segment::new(classify_shell(grep).source(), grep)],
        );
        assert_eq!(verdict(&read, &targets(&paths)).0, Citation::CitedRead);

        // An unrecognized command naming the file stays at the weak tier.
        let odd = format!("my-tool --input {DOSSIER}");
        let ambiguous = view(
            Fidelity::Full,
            vec![Segment::new(classify_shell(&odd).source(), odd.as_str())],
        );
        assert_eq!(
            verdict(&ambiguous, &targets(&paths)).0,
            Citation::CitedMention
        );
    }

    #[test]
    fn multibyte_text_does_not_split_a_char_boundary() {
        let paths = vec![DOSSIER.to_string()];
        let text = "— déjà vu — docs/features/oauth-login.md — ✅";
        let v = view(
            Fidelity::Full,
            vec![Segment::new(SegmentSource::AgentProse, text)],
        );
        assert_eq!(verdict(&v, &targets(&paths)).0, Citation::CitedMention);
    }
}
