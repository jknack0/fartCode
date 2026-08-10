//! Feature dossiers — per-feature project memory (E19-01, #70; ADR-0038
//! items 1–2).
//!
//! One dossier per feature at `docs/features/<slug>.md`, born INSIDE the
//! worktree at the card's first `agent_step` entry so the file rides the
//! feature branch and lands with the code (ADR-0038 item 5, "merge is
//! publication").
//!
//! This module owns the *content and file* half: the slug, the backfilled
//! header, and the append primitives. The *lifecycle* half (consent,
//! worktree resolution, `issues.dossier_path`, event subscription) lives in
//! `fartcode-app::dossiers`, where the App's services are wired.
//!
//! Three rules the whole feature rests on:
//!
//! - **Append-only, never rewrite.** The app owns exactly one section —
//!   `## Timeline` — and inserts lines at its end. Agent-written sections
//!   (`## <Column> — <date>`, ADR-0038 item 2, seeded by #71) are never
//!   read, reordered, or touched. Merge conflicts stay trivial because no
//!   two writers edit the same lines.
//! - **Memory, never a gate.** Every entry point here returns a `Result`
//!   the caller is expected to log and drop. A dossier that cannot be
//!   written must not stop an agent from running.
//! - **Only ever touch our own files.** `docs/features/` is a common
//!   hand-written convention, so "the path exists" is never taken as
//!   permission. A file is written to only when it carries
//!   [`DOSSIER_MARKER`] (or a Timeline section) AND names this card;
//!   anything else makes the card step aside onto a disambiguated path.
//!   A human's document is never opened for writing, never appended to,
//!   never adopted.
//!
//! **Nothing here trusts card text.** Title, body, acceptance criteria and
//! PRD fields are user-controlled and are copied into the file, so they are
//! heading-demoted ([`demote_headings`]) or flattened to one line
//! ([`inline`]) on the way in. Otherwise a card body containing a literal
//! `## Timeline` line would capture every future breadcrumb — the
//! structure of a markdown file the app parses is part of its trust
//! boundary.

use std::path::{Path, PathBuf};

use crate::issues::Issue;
use crate::tasks::naming::{generate_task_name, random_suffix};
use crate::Error;

/// Repo-relative directory every dossier lives in (ADR-0038 item 1).
pub const DOSSIER_DIR: &str = "docs/features";

/// The one heading the app writes under. Matched exactly (trailing
/// whitespace trimmed), so an agent section that merely starts with the
/// word is not mistaken for it.
pub const TIMELINE_HEADING: &str = "## Timeline";

/// Proof a file is a fartCode dossier rather than a document that merely
/// shares its path. Written into the header at birth; checked before any
/// adoption or append.
pub const DOSSIER_MARKER: &str = "<!-- fartCode feature dossier";

/// The sections the APP writes — [`backfilled_header`]'s skeleton plus
/// `## Timeline`. Everything else in a dossier is the agent's own words.
///
/// ADR-0038 item 2 draws exactly this line ("app writes the skeleton;
/// agents write the substance"), and item 4 indexes the substance: the
/// ⌘K indexer (E19-03) skips these four. Kept HERE, beside the writer
/// that emits them, so a new header section cannot start leaking into
/// search without this list being looked at.
///
/// Matched exactly, so an agent section that merely starts with one of
/// these words (`## Context — 2026-08-09`) is still indexed.
pub const APP_SECTIONS: &[&str] = &["Context", "Acceptance", "References", "Timeline"];

/// Whether a `## ` heading names an app-written section ([`APP_SECTIONS`]).
pub fn is_app_section(heading: &str) -> bool {
    APP_SECTIONS.contains(&heading.trim())
}

/// Machine anchor for the Timeline section, written directly under the
/// heading. The append point is resolved by THIS, not by the visible
/// heading text: card-supplied text is heading-demoted on the way in, but
/// the sentinel makes the anchor unforgeable rather than merely unlikely
/// to be forged.
pub const TIMELINE_SENTINEL: &str = "<!-- fartcode:timeline -->";

/// How many times [`append_timeline`] will recompute after losing a race
/// with another writer before giving up. Giving up is correct: the
/// alternative is clobbering whatever the other writer just produced.
const APPEND_ATTEMPTS: u32 = 4;

/// How many disambiguated filenames [`place_dossier`] will try before
/// declining to create one.
const PLACEMENT_ATTEMPTS: usize = 8;

/// How the card got onto the board. The app records no explicit source
/// column, so provenance is DERIVED from the fields each entry path fills
/// (E19-01 decision — see [`provenance`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// Imported from an external tracker; carries the source URL
    /// (`issues.external_ref`, written only by the GitHub importer).
    Import(String),
    /// Applied from a PM proposal; carries the PRD path the breakdown
    /// referenced.
    Proposal(String),
    /// Typed in by hand — also the honest answer for a PRD-less proposal
    /// (see [`provenance`]).
    Manual,
}

impl Provenance {
    /// One-line rendering for the dossier header/timeline.
    pub fn label(&self) -> String {
        match self {
            Provenance::Import(url) => format!("import · {url}"),
            Provenance::Proposal(prd) => format!("proposal · {prd}"),
            Provenance::Manual => "manual".to_string(),
        }
    }
}

/// Derives the card's provenance from what the entry paths actually store.
///
/// **Spec resolution (E19-01):** ADR-0038 asks the header to record
/// "proposal / import / manual", but no column records it. The three entry
/// paths are distinguishable only by their side effects:
/// `github::import` is the sole writer of `external_ref`, and
/// `issue_proposal::apply_proposal` is the only path that fills `prd_path`
/// at creation (the manual add dialog and the `issue_create` command leave
/// it unset). So: external_ref → import, else prd_path → proposal, else
/// manual.
///
/// The one imprecision, stated rather than hidden: a proposal whose block
/// carried no `prd` is indistinguishable from a manual add and reads as
/// `manual`. Recording provenance explicitly is a column, not a heuristic —
/// deliberately not added here, because nothing but this header wants it
/// yet.
pub fn provenance(issue: &Issue) -> Provenance {
    if let Some(url) = issue.external_ref.as_deref().filter(|s| !s.is_empty()) {
        return Provenance::Import(url.to_string());
    }
    if let Some(prd) = issue.prd_path.as_deref().filter(|s| !s.is_empty()) {
        return Provenance::Proposal(prd.to_string());
    }
    Provenance::Manual
}

/// The dossier slug — "the same way task names do" (ADR-0038 item 1):
/// [`generate_task_name`], the exact call
/// `create_task_params` makes to derive a task's branch name from an issue
/// title. No second slugifier exists.
///
/// `auto_generate: false` because a random `happy-otters-sing` dossier
/// filename would be worse than a stable fallback: a title that sanitizes
/// to nothing (all punctuation, all emoji) falls back to the issue id,
/// which is unique and greppable.
pub fn dossier_slug(issue: &Issue) -> String {
    let slug = generate_task_name(Some(&issue.title), None, false);
    if slug.is_empty() {
        // `iss_<uuid>` is already `[a-z0-9_-]`; sanitize for the `_`.
        generate_task_name(Some(&issue.id), None, false)
    } else {
        slug
    }
}

/// Repo-relative dossier path for a card, e.g.
/// `docs/features/oauth-login.md`. Always forward slashes — this string is
/// stored in `issues.dossier_path` and rendered as a link, so it is a repo
/// path, not a host path.
pub fn dossier_relative_path(issue: &Issue) -> String {
    format!("{DOSSIER_DIR}/{}.md", dossier_slug(issue))
}

/// `YYYY-MM-DD HH:MM` in UTC — the Timeline date prefix (handoff v3 §8f).
/// Derived without a date crate: the workspace has none, and a dossier
/// breadcrumb does not justify one.
fn format_stamp(unix_secs: i64) -> String {
    // Days since the Unix epoch → civil date (Howard Hinnant's algorithm).
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60
    )
}

/// Now, as a Timeline stamp.
pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_stamp(secs)
}

/// One Timeline entry: `- <stamp> · <fact>`. The fact is flattened to one
/// line so a breadcrumb can never introduce structure.
pub fn timeline_line(fact: &str) -> String {
    format!("- {} · {}", now_stamp(), inline(fact))
}

/// Flattens user text to a single line for inline rendering (titles, PRD
/// paths and sections, tracker URLs, column and blocker names). Any
/// embedded newline would otherwise let a field start a line — and a line
/// is all it takes to forge a heading.
pub fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = c == ' ';
        }
    }
    out.trim().to_string()
}

/// Demotes every ATX heading in copied card text by one level, so quoted
/// user content cannot emit a `## ` line.
///
/// This protects two parsers at once: the Timeline anchor (a body-borne
/// `## Timeline` would otherwise capture the append point) and the
/// section-end scan (a body-borne `## anything` would truncate the
/// Timeline block early). Demoting rather than escaping keeps the text
/// readable and the document's outline intact — the user's `# Overview`
/// still renders as a heading, one level down, nested under the section
/// that quotes it, which is where it belongs.
///
/// **Fences are structure too, since E19-03.** The section scan
/// ([`section_heading_lines`]) skips headings inside fenced code blocks, so
/// a card body that OPENS a fence and never closes it — a pasted stack
/// trace beginning ```` ```text ```` is enough — would make every heading
/// below the quote invisible. So an unbalanced fence is CLOSED here.
///
/// Closing rather than mangling is deliberate: it preserves the user's
/// bytes exactly (their code block simply ends where the quote does, which
/// is also how it should render), where escaping or indenting the fence
/// markers would corrupt a legitimate paste. The neutralization the
/// structure needs is "the fence does not escape the quote", not "the
/// fence characters are gone".
pub fn demote_headings(text: &str) -> String {
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            if line.trim_start().starts_with('#') && line.starts_with('#') {
                format!("#{line}")
            } else {
                line.to_string()
            }
        })
        .collect();
    // Belt to the anchor scan's braces: even with the append point scanned
    // from the sentinel, an unclosed fence here would swallow the header's
    // own `## Acceptance` / `## References` for every other reader.
    let closer = {
        let lines: Vec<&str> = out.iter().map(String::as_str).collect();
        scan_sections(&lines)
            .1
            .map(|(marker, run)| marker.to_string().repeat(run))
    };
    if let Some(closer) = closer {
        out.push(closer);
    }
    out.join("\n")
}

/// The `- card:` line that proves a dossier belongs to a given card. The
/// ownership check for adoption keys off this exact string.
pub fn card_marker(issue_id: &str) -> String {
    format!("- card: `{issue_id}`")
}

/// The backfilled header the dossier is born with (ADR-0038 item 1): issue
/// title/body/acceptance, the PRD reference, provenance, and the
/// pre-worktree timeline reconstructed from what the DB already holds.
///
/// **What the pre-worktree timeline can and cannot say.** The app stores
/// no board-move history — only `created_at`, the CURRENT column, and the
/// live blocker edges. So the backfill records creation (with its source),
/// the column the card was sitting in when the worktree provisioned, and
/// the blockers standing at that moment. Intermediate lane moves before
/// the first dispatch are not recoverable and are not invented.
///
/// `column_name` is the column whose entry provisioned the worktree
/// (`None` when the caller could not resolve it — the line is then
/// omitted, never faked).
pub fn backfilled_header(issue: &Issue, column_name: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&inline(&issue.title));
    out.push_str("\n\n");
    out.push_str(DOSSIER_MARKER);
    out.push_str(
        " (ADR-0038). Append-only: add sections, never rewrite existing \
         ones. The app owns `## Timeline`; agents add `## <Column> — <date>` \
         sections below it. -->\n\n",
    );

    // Card text is quoted, never trusted: headings are demoted so user
    // content cannot forge the structure this file's parsers key off.
    if let Some(body) = issue
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        out.push_str("## Context\n\n");
        out.push_str(&demote_headings(body));
        out.push_str("\n\n");
    }

    if !issue.acceptance.is_empty() {
        out.push_str("## Acceptance\n\n");
        for item in &issue.acceptance {
            out.push_str("- ");
            out.push_str(&inline(item));
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## References\n\n");
    out.push_str(&card_marker(&issue.id));
    out.push('\n');
    out.push_str(&format!("- source: {}\n", provenance(issue).label()));
    if let Some(prd) = issue.prd_path.as_deref().filter(|p| !p.is_empty()) {
        match issue.prd_section.as_deref().filter(|s| !s.is_empty()) {
            Some(section) => {
                out.push_str(&format!("- PRD: `{}` — {}\n", inline(prd), inline(section)))
            }
            None => out.push_str(&format!("- PRD: `{}`\n", inline(prd))),
        }
    }
    if let Some(url) = issue.external_ref.as_deref().filter(|u| !u.is_empty()) {
        out.push_str(&format!("- tracker: {}\n", inline(url)));
    }
    out.push('\n');

    // Heading + machine sentinel: the append point is resolved by the
    // sentinel, so it survives an agent retitling the heading and cannot
    // be captured by text quoted above.
    out.push_str(TIMELINE_HEADING);
    out.push('\n');
    out.push_str(TIMELINE_SENTINEL);
    out.push_str("\n\n");
    let created = inline(
        issue
            .created_at
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or(&now_stamp()),
    );
    out.push_str(&format!(
        "- {created} · created · {}\n",
        provenance(issue).label()
    ));
    if !issue.blockers.is_empty() {
        let open: Vec<String> = issue
            .blockers
            .iter()
            .filter(|b| !b.counts_as_done)
            .map(|b| inline(&b.title))
            .collect();
        if !open.is_empty() {
            out.push_str(&format!("- {created} · blocked by: {}\n", open.join(", ")));
        }
    }
    match column_name {
        Some(name) => out.push_str(&timeline_line(&format!(
            "dossier created with the worktree · {}",
            inline(name)
        ))),
        None => out.push_str(&timeline_line("dossier created with the worktree")),
    }
    out.push('\n');
    out
}

/// What is already sitting at a candidate dossier path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupant {
    /// Nothing there — free to write.
    Free,
    /// A fartCode dossier naming THIS card. Adopt it: a re-provisioned
    /// card keeps the history it already accumulated.
    OurDossier,
    /// A fartCode dossier naming a DIFFERENT card — two same-titled cards
    /// racing for one slug. Step aside; interleaving two features'
    /// histories in one file helps neither.
    OtherDossier,
    /// Anything else: a human's document that happens to live at the path
    /// our slug produced (`docs/features/` is a common hand-written
    /// convention). Never read for adoption, never written to, never
    /// appended to.
    Foreign,
}

/// Classifies whatever occupies `path` with respect to `issue_id`.
///
/// Recognition is deliberately narrow. A file counts as a dossier only if
/// it carries [`DOSSIER_MARKER`] or a real Timeline section, and counts as
/// OURS only if it also carries this card's [`card_marker`]. An unreadable
/// file (permissions, binary, mid-write) is [`Occupant::Foreign`] — the
/// safe answer when we cannot tell.
pub fn inspect(path: &Path, issue_id: &str) -> Occupant {
    if !path.exists() {
        return Occupant::Free;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return Occupant::Foreign;
    };
    let looks_like_a_dossier = content.contains(DOSSIER_MARKER)
        || timeline_anchor(&content.lines().collect::<Vec<_>>()).is_some();
    if !looks_like_a_dossier {
        return Occupant::Foreign;
    }
    if content.contains(&card_marker(issue_id)) {
        Occupant::OurDossier
    } else {
        Occupant::OtherDossier
    }
}

/// Where a card's dossier goes, and whether this call created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Repo-relative path, for `issues.dossier_path`.
    pub rel_path: String,
    /// `true` when this call wrote the header; `false` when it adopted the
    /// card's own existing dossier.
    pub created: bool,
}

/// Places the card's dossier inside `worktree`, creating or adopting, and
/// never touching a file that is not ours.
///
/// Candidate order: the path already recorded on the card (so a
/// re-provisioned card returns to its own file even if the title changed),
/// then `<slug>.md`, then `<slug>-<short card id>.md`, then randomly
/// suffixed variants. The first candidate that is [`Occupant::Free`] gets
/// the header; the first that is [`Occupant::OurDossier`] is adopted
/// as-is. `OtherDossier` and `Foreign` move to the next candidate — that
/// is the whole point: a hand-written `docs/features/dark-mode.md` must
/// survive a card called "Dark mode" untouched.
pub fn place_dossier(
    worktree: &Path,
    issue: &Issue,
    header: &str,
    stored_rel: Option<&str>,
) -> Result<Placement, Error> {
    for candidate in candidate_paths(issue, stored_rel) {
        let target = worktree_file(worktree, &candidate);
        match inspect(&target, &issue.id) {
            Occupant::OurDossier => {
                return Ok(Placement {
                    rel_path: candidate,
                    created: false,
                })
            }
            Occupant::Free => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                atomic_write(&target, header)?;
                return Ok(Placement {
                    rel_path: candidate,
                    created: true,
                });
            }
            // Someone else's file. Try the next name; never clobber.
            Occupant::OtherDossier | Occupant::Foreign => continue,
        }
    }
    Err(Error::Internal(format!(
        "no free dossier path for issue {} after {PLACEMENT_ATTEMPTS} attempts",
        issue.id
    )))
}

/// Candidate filenames in priority order (see [`place_dossier`]).
fn candidate_paths(issue: &Issue, stored_rel: Option<&str>) -> Vec<String> {
    let slug = dossier_slug(issue);
    let short = short_id(&issue.id);
    let mut out: Vec<String> = Vec::with_capacity(PLACEMENT_ATTEMPTS + 2);
    if let Some(stored) = stored_rel.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(stored.to_string());
    }
    out.push(format!("{DOSSIER_DIR}/{slug}.md"));
    // Deterministic first, so two same-titled cards land on stable,
    // greppable names rather than a pair of random ones.
    out.push(format!("{DOSSIER_DIR}/{slug}-{short}.md"));
    while out.len() < PLACEMENT_ATTEMPTS {
        out.push(format!(
            "{DOSSIER_DIR}/{slug}-{short}-{}.md",
            random_suffix()
        ));
    }
    out.dedup();
    out
}

/// First 8 sanitized chars of the card id — enough to disambiguate two
/// same-titled cards, short enough to keep the filename readable.
fn short_id(issue_id: &str) -> String {
    let sanitized = generate_task_name(Some(issue_id), None, false);
    // `iss_<uuid>` sanitizes to `iss-<uuid>`; skip the constant prefix so
    // the 8 chars carry actual entropy.
    let tail = sanitized.strip_prefix("iss-").unwrap_or(&sanitized);
    tail.chars().take(8).collect()
}

/// Appends one line to the dossier's `## Timeline` section.
///
/// `once_key`, when set, makes the append idempotent: if a Timeline line
/// already ENDS WITH that key the call is a no-op returning `false`. It
/// exists for facts that are true once but arrive repeatedly — `PrUpdated`
/// fires on every check/comment refresh, not only on open and merge. The
/// match is line-suffix rather than substring so `…/pull/1` cannot be
/// found inside an existing `…/pull/12`.
///
/// Everything outside the Timeline section is preserved byte for byte: the
/// insert point is the last non-blank line of the Timeline block, found
/// from the sentinel and bounded by the next `## ` heading. Agent sections
/// are never parsed, moved, or rewritten.
///
/// **Durability.** The rewrite is temp-file + rename, so a crash leaves
/// either the old file or the new one and never a truncated corpse (which
/// adoption would then inherit). The read-modify-write window is narrowed
/// by re-stat'ing before the rename and recomputing when the file moved
/// under us; after [`APPEND_ATTEMPTS`] losses it errors rather than
/// overwriting a concurrent writer's work. This is not a lock — an agent
/// editing the file in the same millisecond can still lose its edit to the
/// rename — but it converts the failure mode from "dossier destroyed" to
/// "one breadcrumb missing", which is the trade ADR-0038 asks for.
///
/// Errors when the file does not exist — a missing dossier means the
/// worktree was torn down (or the branch deleted), and ADR-0038 item 2 says
/// post-teardown events go unrecorded. Callers log and drop.
pub fn append_timeline(
    worktree: &Path,
    rel_path: &str,
    line: &str,
    once_key: Option<&str>,
) -> Result<bool, Error> {
    let target = worktree_file(worktree, rel_path);
    for _ in 0..APPEND_ATTEMPTS {
        let before = fingerprint(&target)?;
        let content = std::fs::read_to_string(&target)?;
        if let Some(key) = once_key {
            if already_recorded(&content, key) {
                return Ok(false);
            }
        }
        let next = insert_under_timeline(&content, line);
        if fingerprint(&target)? != before {
            continue; // changed under us — re-read rather than clobber
        }
        atomic_write(&target, &next)?;
        return Ok(true);
    }
    Err(Error::Internal(format!(
        "dossier {rel_path} kept changing under the append — breadcrumb dropped \
         rather than overwrite a concurrent writer"
    )))
}

/// True when some Timeline line already ends with `key`. Line-anchored on
/// purpose: a bare `contains` would treat `pr merged · …/pull/1` as
/// already present because `…/pull/12` contains it.
fn already_recorded(content: &str, key: &str) -> bool {
    content.lines().any(|l| l.trim_end().ends_with(key))
}

/// Cheap change detector for the read-modify-write window: size plus mtime.
/// Coarse mtime resolution means this can miss a same-millisecond
/// same-length edit; it is a narrowing, not a guarantee (see
/// [`append_timeline`]).
fn fingerprint(path: &Path) -> Result<(u64, Option<std::time::SystemTime>), Error> {
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), meta.modified().ok()))
}

/// Writes via a uniquely-named temp file in the SAME directory + rename,
/// so the replacement is atomic on the target filesystem and two
/// concurrent writers can never share a temp path. **Every** failure path
/// cleans up after itself rather than littering the user's repo — the write
/// as well as the rename (a partial write left a `.tmp` corpse behind
/// before the E19-02 fix round, and `crate::skills` reads directory
/// contents to decide ownership, so debris there is not merely untidy: it
/// makes our own scaffold look like somebody else's).
///
/// Crate-visible since E19-02 (#71): `crate::skills` writes into the same
/// repositories under the same contract, and a second implementation of
/// "durably replace a file in a stranger's checkout" is a second place to
/// get it wrong.
pub(crate) fn atomic_write(target: &Path, content: &str) -> Result<(), Error> {
    let dir = target
        .parent()
        .ok_or_else(|| Error::Internal(format!("dossier path has no parent: {target:?}")))?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("dossier.md");
    let tmp = dir.join(temp_name(name));
    if let Err(e) = std::fs::write(&tmp, content) {
        // A partially written temp file is still a file: remove it, or a
        // disk-full write poisons the directory's ownership check forever.
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Io(e));
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Io(e));
    }
    Ok(())
}

/// The temp filename [`atomic_write`] uses for `name`. Hidden (leading dot)
/// and `.tmp`-suffixed so [`is_temp_name`] can recognize it.
fn temp_name(name: &str) -> String {
    format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple())
}

/// True when `name` looks like one of OUR temp files. The one definition,
/// shared with [`temp_name`], so a debris file can never be mistaken for a
/// user's document (or vice versa).
pub(crate) fn is_temp_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

/// Joins a repo-relative dossier path onto a worktree root. The path is
/// app-generated (`docs/features/<sanitized slug>.md`), never user input,
/// so there is no traversal surface to guard here — the slug's charset is
/// `[a-z0-9-]` by construction.
fn worktree_file(worktree: &Path, rel_path: &str) -> PathBuf {
    let mut path = worktree.to_path_buf();
    for segment in rel_path.split('/').filter(|s| !s.is_empty()) {
        path.push(segment);
    }
    path
}

// ---------------------------------------------------------------------------
// Section parsing — ONE parser, shared
// ---------------------------------------------------------------------------

/// One `## ` block of a dossier: the heading text and everything under it
/// up to the next `## ` heading (or EOF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading text with the `## ` marker and surrounding whitespace
    /// stripped, e.g. `Plan — 2026-08-09`.
    pub heading: String,
    /// The block's contents, trimmed. Never includes the heading line.
    pub body: String,
}

/// A fence opener/closer on this line, as `(char, run length)`.
///
/// CommonMark-lite: three or more backticks or tildes, optionally
/// indented. Enough to keep a fenced ```` ```md ```` sample containing
/// `## Plan` from reading as document structure, which is the whole job.
fn fence_run(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    (run >= 3).then_some((marker, run))
}

/// Line indices of every REAL `## ` heading, skipping anything inside a
/// fenced code block.
///
/// **The only place a dossier's structure is decided.** Both the Timeline
/// append point ([`insert_under_timeline`]) and the ⌘K section indexer
/// (E19-03) resolve headings through this, so the file the appender writes
/// into and the file search reads can never disagree about where a section
/// begins.
///
/// `## ` requires the trailing space, so `### Subheading` is not a
/// section — a subheading belongs to the section above it. Line-initial
/// only, matching [`demote_headings`]'s trust boundary: an indented
/// `  ## x` inside quoted card text is not structure.
pub fn section_heading_lines(lines: &[&str]) -> Vec<usize> {
    section_heading_lines_checked(lines).0
}

/// [`section_heading_lines`] plus the fact callers need to act on: whether
/// the scan ENDED INSIDE an unclosed fence.
///
/// A fence with no closer swallows every heading below it — the same thing
/// a markdown renderer does, so the parse is not "wrong". But it means the
/// empty tail is an artifact of one stray line, not evidence that the
/// sections are gone, and a caller that PRUNES on absence (the ⌘K indexer)
/// must be able to tell those apart. It is the same distinction the app
/// draws for an unreadable file: never delete a list you could not confirm
/// is gone.
pub fn section_heading_lines_checked(lines: &[&str]) -> (Vec<usize>, bool) {
    let (heads, open) = scan_sections(lines);
    (heads, open.is_some())
}

/// The scan itself: heading lines, plus the fence left open at the end (as
/// `(char, run)` so a caller can emit a matching closer).
fn scan_sections(lines: &[&str]) -> (Vec<usize>, Option<(char, usize)>) {
    let mut out = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        match open {
            // Inside a fence: only a matching closer (same char, at least
            // as long, no info string) gets us out.
            Some((open_char, open_run)) => {
                if let Some((marker, run)) = fence_run(line) {
                    let rest = line.trim_start().trim_start_matches(marker).trim();
                    if marker == open_char && run >= open_run && rest.is_empty() {
                        open = None;
                    }
                }
            }
            None => {
                if let Some(fence) = fence_run(line) {
                    open = Some(fence);
                } else if line.starts_with("## ") {
                    out.push(i);
                }
            }
        }
    }
    (out, open)
}

/// Splits a dossier into its `## ` sections (see [`section_heading_lines`]).
///
/// CRLF-safe by construction: `str::lines` strips the carriage return, and
/// heading/body text is trimmed. A file with no `## ` heading yields an
/// empty vec.
pub fn sections(content: &str) -> Vec<Section> {
    sections_checked(content).0
}

/// [`sections`] plus [`section_heading_lines_checked`]'s unclosed-fence
/// flag — `true` means "sections below here may have been swallowed", not
/// "there are no more sections".
pub fn sections_checked(content: &str) -> (Vec<Section>, bool) {
    let lines: Vec<&str> = content.lines().collect();
    let (heads, ended_open) = section_heading_lines_checked(&lines);
    let sections = heads
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = heads.get(n + 1).copied().unwrap_or(lines.len());
            Section {
                // `## ` is ASCII, so the byte slice is char-safe.
                heading: lines[start][3..].trim().to_string(),
                body: lines[start + 1..end].join("\n").trim().to_string(),
            }
        })
        .collect();
    (sections, ended_open)
}

/// Resolves the Timeline anchor: the line index the block starts after.
///
/// Sentinel FIRST — [`TIMELINE_SENTINEL`] is machine-written and unforgeable
/// by card text. Falling back to the visible heading, `rposition` (not
/// `position`) is deliberate: on a pre-sentinel dossier, or one an agent
/// hand-edited, the LAST `## Timeline` wins, so a copy quoted earlier in the
/// document cannot capture the append point. Returns `None` when the file
/// has no Timeline section at all.
fn timeline_anchor(lines: &[&str]) -> Option<usize> {
    lines
        .iter()
        .rposition(|l| l.trim() == TIMELINE_SENTINEL)
        .or_else(|| lines.iter().rposition(|l| l.trim_end() == TIMELINE_HEADING))
}

/// Where the Timeline block ENDS, scanned from its anchor: the next REAL
/// `## ` heading (the first agent-written section) or EOF.
///
/// **Scanned FROM THE ANCHOR with fresh fence state**, which is the whole
/// trust boundary: everything above the sentinel includes quoted card text,
/// and a card body that opens a fence and never closes it (a pasted stack
/// trace starting ```` ```text ```` is enough) would otherwise make the scan
/// treat the entire rest of the document as code — no `## ` heading found,
/// `end` falling back to EOF. The sentinel is machine-written territory; a
/// fence opened in user text above it does not reach past it.
///
/// `### ` subheadings do not end the block, and neither does a `## ` line
/// inside a fenced code block — the same scan the ⌘K indexer uses, so
/// appends, search and the card detail agree on where the block stops.
fn timeline_end(lines: &[&str], start: usize) -> usize {
    section_heading_lines(&lines[start..])
        .into_iter()
        .map(|i| i + start)
        .find(|i| *i > start)
        .unwrap_or(lines.len())
}

/// The app's `## Timeline` block, resolved the SAME way the appender
/// resolves its insert point ([`timeline_anchor`] — sentinel first, the
/// last matching heading only as the pre-sentinel fallback).
///
/// Added for the card detail (E19-06, #75), and deliberately here rather
/// than in the viewer: resolving the block by heading TEXT is a different
/// answer from resolving it by anchor, and the two disagree exactly where
/// it hurts. An agent that documents the convention with a bare
/// `## Timeline` line in prose steals a text-matched block — so the card
/// would render an empty timeline while every append kept landing in the
/// real one. And when an agent RETITLES the heading, a case the sentinel
/// exists to survive, a text match finds nothing at all. One anchor, three
/// readers.
///
/// The returned [`Section::heading`] is the block's real heading text
/// (whatever it has been retitled to), and the body excludes the sentinel
/// line. `None` when the file has no Timeline section at all.
pub fn timeline_block(content: &str) -> Option<Section> {
    let lines: Vec<&str> = content.lines().collect();
    let start = timeline_anchor(&lines)?;
    let end = timeline_end(&lines, start);
    // The anchor is usually the sentinel, which sits UNDER the heading —
    // walk back to the heading the block actually carries.
    let heading = lines[..=start]
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("## "))
        .unwrap_or(TIMELINE_HEADING.trim_start_matches("## "))
        .trim()
        .to_string();
    Some(Section {
        heading,
        body: lines[start + 1..end].join("\n").trim().to_string(),
    })
}

/// Pure text surgery behind [`append_timeline`] — the whole append-safety
/// contract lives here, so it is unit-testable without a filesystem.
fn insert_under_timeline(content: &str, line: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    let Some(start) = timeline_anchor(&lines) else {
        // No Timeline section: an agent removed it. Start one at the end
        // rather than guessing where it belonged — appending is always safe.
        // (Only reachable on a file already proven ours: `inspect` keeps
        // foreign documents out of this path entirely.)
        let mut out = content.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(TIMELINE_HEADING);
        out.push('\n');
        out.push_str(TIMELINE_SENTINEL);
        out.push_str("\n\n");
        out.push_str(line);
        out.push('\n');
        return out;
    };

    // The block's extent — shared with the card-detail reader
    // ([`timeline_block`]), so the lines an append lands among are exactly
    // the lines the card renders.
    let end = timeline_end(&lines, start);

    // Back over the blank lines that separate the block from what follows,
    // so the new entry joins the list instead of floating after it.
    let mut at = end;
    while at > start + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }

    let mut out: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    out.extend_from_slice(&lines[..at]);
    out.push(line);
    out.extend_from_slice(&lines[at..]);
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issues::{BlockerRef, Lane};

    fn issue(title: &str) -> Issue {
        Issue {
            id: "iss_1".into(),
            project_id: "p1".into(),
            title: title.into(),
            body: Some("why it matters".into()),
            acceptance: vec!["it works".into()],
            lane: Lane::InProgress,
            position: 0,
            provider: None,
            model: None,
            prd_path: None,
            prd_section: None,
            dossier_path: None,
            linked_task_id: None,
            external_ref: None,
            column_id: None,
            blocked: false,
            blockers: Vec::new(),
            created_at: Some("2026-08-01 09:00:00".into()),
            updated_at: None,
        }
    }

    #[test]
    fn slug_matches_the_task_name_helper() {
        let i = issue("Implement OAuth login!");
        assert_eq!(dossier_slug(&i), "implement-oauth-login");
        assert_eq!(
            dossier_relative_path(&i),
            "docs/features/implement-oauth-login.md"
        );
        // Same slugifier the branch name uses — not a second one.
        assert_eq!(
            dossier_slug(&i),
            generate_task_name(Some("Implement OAuth login!"), None, false)
        );
    }

    #[test]
    fn unsluggable_title_falls_back_to_the_card_id() {
        let i = issue("🙂🙂🙂");
        assert_eq!(dossier_slug(&i), "iss-1");
    }

    #[test]
    fn provenance_reads_import_then_proposal_then_manual() {
        let mut i = issue("x");
        assert_eq!(provenance(&i), Provenance::Manual);
        i.prd_path = Some("docs/prds/oauth.md".into());
        assert_eq!(
            provenance(&i),
            Provenance::Proposal("docs/prds/oauth.md".into())
        );
        i.external_ref = Some("https://github.com/o/r/issues/7".into());
        assert_eq!(
            provenance(&i),
            Provenance::Import("https://github.com/o/r/issues/7".into())
        );
    }

    #[test]
    fn header_backfills_body_acceptance_prd_and_timeline() {
        let mut i = issue("Implement OAuth login");
        i.prd_path = Some("docs/prds/oauth.md".into());
        i.prd_section = Some("## Flow".into());
        i.blockers = vec![
            BlockerRef {
                id: "iss_0".into(),
                title: "Pick a provider".into(),
                lane: Lane::Backlog,
                counts_as_done: false,
                column_id: None,
            },
            BlockerRef {
                id: "iss_9".into(),
                title: "Already landed".into(),
                lane: Lane::Done,
                counts_as_done: true,
                column_id: None,
            },
        ];
        let header = backfilled_header(&i, Some("In Progress"));

        assert!(header.starts_with("# Implement OAuth login\n"));
        assert!(header.contains("why it matters"));
        assert!(header.contains("- it works"));
        assert!(header.contains("- PRD: `docs/prds/oauth.md` — ## Flow"));
        assert!(header.contains("- source: proposal · docs/prds/oauth.md"));
        assert!(header.contains("## Timeline"));
        assert!(header.contains("2026-08-01 09:00:00 · created · proposal"));
        // Only OPEN blockers are pre-worktree history worth recording.
        assert!(header.contains("blocked by: Pick a provider"));
        assert!(!header.contains("Already landed"));
        assert!(header.contains("dossier created with the worktree · In Progress"));
    }

    #[test]
    fn append_lands_under_timeline_and_leaves_agent_sections_intact() {
        let file = "# Feature\n\n## Timeline\n\n- 2026-08-09 10:00 · created · manual\n\n\
                    ## Plan — 2026-08-09\n\nWe chose X over Y because Z.\n\n\
                    ## Implement — 2026-08-09\n\nDone.\n";
        let out = insert_under_timeline(file, "- 2026-08-09 11:00 · In Progress · settled");

        let timeline_end = out.find("## Plan").unwrap();
        assert!(out[..timeline_end].contains("In Progress · settled"));
        assert!(out.contains("## Plan — 2026-08-09\n\nWe chose X over Y because Z."));
        assert!(out.contains("## Implement — 2026-08-09\n\nDone."));
        // Order preserved, nothing reshuffled.
        assert!(out.find("## Plan").unwrap() < out.find("## Implement").unwrap());
    }

    #[test]
    fn append_with_no_timeline_section_starts_one_at_the_end() {
        let file = "# Feature\n\n## Plan — 2026-08-09\n\nStuff.\n";
        let out = insert_under_timeline(file, "- x · y");
        assert!(out.contains("## Plan — 2026-08-09\n\nStuff."));
        assert!(out.trim_end().ends_with("- x · y"));
        assert!(out.find("## Plan").unwrap() < out.find(TIMELINE_HEADING).unwrap());
    }

    #[test]
    fn append_at_eof_when_timeline_is_the_last_section() {
        let file = "# Feature\n\n## Timeline\n\n- a\n";
        let out = insert_under_timeline(file, "- b");
        assert_eq!(out, "# Feature\n\n## Timeline\n\n- a\n- b\n");
    }

    #[test]
    fn subheadings_do_not_end_the_timeline_block() {
        let file = "## Timeline\n\n- a\n\n### note\n\n- b\n\n## Plan — x\n\ncontent\n";
        let out = insert_under_timeline(file, "- c");
        let plan = out.find("## Plan").unwrap();
        assert!(out[..plan].contains("- c"));
        assert!(out.contains("### note"));
    }

    /// The Timeline block used to end at ANY line starting `## `,
    /// including one inside a fenced sample the seeded skill's own docs
    /// would contain — breadcrumbs then landed above the sample instead of
    /// at the end of the list. Sharing [`section_heading_lines`] with the
    /// E19-03 indexer fixed both parsers at once.
    #[test]
    fn a_fenced_heading_does_not_end_the_timeline_block() {
        let file = "## Timeline\n\n- a\n\n```md\n## Plan — not a section\n```\n\n- b\n";
        let out = insert_under_timeline(file, "- c");
        assert!(out.trim_end().ends_with("- b\n- c"), "{out}");
        assert!(out.contains("## Plan — not a section"), "sample preserved");
    }

    #[test]
    fn place_creates_then_adopts_the_same_cards_dossier() {
        let tmp = tempfile::tempdir().unwrap();
        let i = issue("Thing");
        let header = backfilled_header(&i, None);

        let first = place_dossier(tmp.path(), &i, &header, None).unwrap();
        assert!(first.created);
        assert_eq!(first.rel_path, "docs/features/thing.md");

        // Re-provision: same card, same file, contents untouched.
        let before = std::fs::read_to_string(tmp.path().join(&first.rel_path)).unwrap();
        let second = place_dossier(tmp.path(), &i, "# SECOND\n", Some(&first.rel_path)).unwrap();
        assert!(!second.created, "adopted, not rewritten");
        assert_eq!(second.rel_path, first.rel_path);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(&second.rel_path)).unwrap(),
            before
        );
    }

    /// The defect that made adoption dangerous: `docs/features/` is a
    /// common hand-written convention, so "a file exists here" is not
    /// permission to write to it.
    #[test]
    fn a_foreign_document_is_never_adopted_or_touched() {
        let tmp = tempfile::tempdir().unwrap();
        const HUMAN_DOC: &str = "# Dark mode\n\nOur design spec. Not a dossier.\n";
        std::fs::create_dir_all(tmp.path().join(DOSSIER_DIR)).unwrap();
        let taken = tmp.path().join(DOSSIER_DIR).join("dark-mode.md");
        std::fs::write(&taken, HUMAN_DOC).unwrap();

        let i = issue("Dark mode");
        assert_eq!(inspect(&taken, &i.id), Occupant::Foreign);

        let placed = place_dossier(tmp.path(), &i, &backfilled_header(&i, None), None).unwrap();
        assert!(placed.created);
        assert_ne!(
            placed.rel_path, "docs/features/dark-mode.md",
            "the card stepped aside onto {}",
            placed.rel_path
        );
        assert_eq!(
            std::fs::read_to_string(&taken).unwrap(),
            HUMAN_DOC,
            "the human's document is byte-identical"
        );

        // …and stays that way once breadcrumbs start flowing.
        append_timeline(tmp.path(), &placed.rel_path, "- later · settled", None).unwrap();
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), HUMAN_DOC);
    }

    #[test]
    fn two_same_titled_cards_get_two_dossiers() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = issue("Dark mode");
        a.id = "iss_aaaaaaaa-1111".into();
        let mut b = issue("Dark mode");
        b.id = "iss_bbbbbbbb-2222".into();

        let pa = place_dossier(tmp.path(), &a, &backfilled_header(&a, None), None).unwrap();
        let pb = place_dossier(tmp.path(), &b, &backfilled_header(&b, None), None).unwrap();
        assert!(pa.created && pb.created);
        assert_ne!(pa.rel_path, pb.rel_path, "histories must not interleave");
        // Each file names its own card.
        assert!(std::fs::read_to_string(tmp.path().join(&pa.rel_path))
            .unwrap()
            .contains(&card_marker(&a.id)));
        assert!(std::fs::read_to_string(tmp.path().join(&pb.rel_path))
            .unwrap()
            .contains(&card_marker(&b.id)));
    }

    #[test]
    fn inspect_classifies_ours_theirs_and_free() {
        let tmp = tempfile::tempdir().unwrap();
        let i = issue("Thing");
        let path = tmp.path().join("d.md");
        assert_eq!(inspect(&path, &i.id), Occupant::Free);

        std::fs::write(&path, backfilled_header(&i, None)).unwrap();
        assert_eq!(inspect(&path, &i.id), Occupant::OurDossier);
        assert_eq!(inspect(&path, "iss_other"), Occupant::OtherDossier);
    }

    #[test]
    fn once_key_makes_an_append_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let i = issue("Thing");
        let placed = place_dossier(tmp.path(), &i, &backfilled_header(&i, None), None).unwrap();
        let key = "pr merged · https://x/pull/1";
        let rel = &placed.rel_path;
        assert!(append_timeline(tmp.path(), rel, &timeline_line(key), Some(key)).unwrap());
        assert!(!append_timeline(tmp.path(), rel, &timeline_line(key), Some(key)).unwrap());
        let on_disk = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert_eq!(on_disk.matches(key).count(), 1);
    }

    /// A bare `contains` treated `…/pull/1` as already recorded because
    /// `…/pull/12` contains it — silently dropping the second PR's line.
    #[test]
    fn once_key_does_not_match_a_longer_line_it_is_a_prefix_of() {
        let content = "## Timeline\n- 2026 · pr merged · https://x/pull/12\n";
        assert!(already_recorded(content, "pr merged · https://x/pull/12"));
        assert!(
            !already_recorded(content, "pr merged · https://x/pull/1"),
            "PR #1 must not be masked by PR #12"
        );
    }

    #[test]
    fn append_to_a_missing_dossier_errors_rather_than_creating_one() {
        let tmp = tempfile::tempdir().unwrap();
        let err = append_timeline(tmp.path(), "docs/features/gone.md", "- x", None);
        assert!(err.is_err(), "a torn-down worktree records nothing");
    }

    #[test]
    fn writes_leave_no_temp_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let i = issue("Thing");
        let placed = place_dossier(tmp.path(), &i, &backfilled_header(&i, None), None).unwrap();
        append_timeline(tmp.path(), &placed.rel_path, "- a · b", None).unwrap();
        let stray: Vec<_> = std::fs::read_dir(tmp.path().join(DOSSIER_DIR))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(stray.is_empty(), "temp files left behind: {stray:?}");
    }

    // -- card text is data, never structure --------------------------------

    /// The forgeable-anchor defect: a body containing a literal
    /// `## Timeline` used to capture every breadcrumb, because the header
    /// copied the body raw and the anchor took the FIRST match.
    #[test]
    fn a_card_body_cannot_forge_the_timeline_anchor() {
        let tmp = tempfile::tempdir().unwrap();
        let mut i = issue("Sneaky");
        i.body = Some("Notes:\n\n## Timeline\n\n- 1999 · fake\n\n## Acceptance\n\nnope".into());
        let header = backfilled_header(&i, None);

        // The body's headings were demoted on the way in.
        assert!(header.contains("### Timeline"), "{header}");
        assert!(!header.contains("\n## Timeline\n- 1999"), "{header}");

        let placed = place_dossier(tmp.path(), &i, &header, None).unwrap();
        append_timeline(tmp.path(), &placed.rel_path, "- now · real", None).unwrap();
        let text = std::fs::read_to_string(tmp.path().join(&placed.rel_path)).unwrap();

        // The breadcrumb landed under the REAL section (after the
        // sentinel), not inside the quoted body.
        let sentinel = text.find(TIMELINE_SENTINEL).unwrap();
        let fake = text.find("- 1999 · fake").unwrap();
        let real = text.find("- now · real").unwrap();
        assert!(fake < sentinel, "the body's copy stays where it was quoted");
        assert!(real > sentinel, "the breadcrumb lands in the real section");
    }

    #[test]
    fn the_sentinel_wins_over_a_stray_heading_and_rposition_is_the_fallback() {
        // Sentinel anchors even when a `## Timeline` line appears earlier.
        let with_sentinel = format!(
            "# F\n\n## Context\n\n## Timeline\n\n- decoy\n\n{TIMELINE_HEADING}\n{TIMELINE_SENTINEL}\n\n- real\n"
        );
        let out = insert_under_timeline(&with_sentinel, "- new");
        let decoy = out.find("- decoy").unwrap();
        let new = out.find("- new").unwrap();
        assert!(
            new > decoy,
            "anchored on the sentinel, not the first heading"
        );

        // Pre-sentinel dossier: the LAST heading wins.
        let legacy = "## Timeline\n\n- decoy\n\n## Plan — x\n\n## Timeline\n\n- real\n";
        let out = insert_under_timeline(legacy, "- new");
        assert!(out.find("- new").unwrap() > out.find("- real").unwrap());
    }

    /// The regression the E19-03 parser factoring introduced: fences became
    /// structural, but `demote_headings` only neutralized `#`. A card body
    /// with an UNCLOSED fence — a pasted stack trace is enough — made the
    /// scan treat the whole rest of the document as code, so no `## `
    /// heading was found, `end` fell back to EOF, and every breadcrumb
    /// landed at the bottom of the file instead of under Timeline.
    #[test]
    fn an_unclosed_fence_in_a_card_body_cannot_swallow_the_document() {
        let tmp = tempfile::tempdir().unwrap();
        let mut i = issue("Crashy");
        i.body = Some("Stack trace:\n\n```text\nthread 'main' panicked at src/main.rs:1".into());
        let header = backfilled_header(&i, None);

        // The quote is closed on the way in, so the header's own structure
        // is still visible to every reader of the file.
        let lines: Vec<&str> = header.lines().collect();
        let headings: Vec<&str> = section_heading_lines(&lines)
            .into_iter()
            .map(|n| lines[n])
            .collect();
        assert!(
            headings.contains(&TIMELINE_HEADING),
            "the Timeline heading survived the quoted fence: {headings:?}"
        );
        assert!(header.contains("thread 'main' panicked"), "bytes preserved");

        // …and a breadcrumb still lands under Timeline, above the agent's
        // sections rather than at EOF.
        let placed = place_dossier(tmp.path(), &i, &header, None).unwrap();
        let path = tmp.path().join(&placed.rel_path);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n## Plan — 2026-08-09\n\nDecided X.\n");
        std::fs::write(&path, text).unwrap();

        append_timeline(tmp.path(), &placed.rel_path, "- now · settled", None).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        let plan = out.find("## Plan").unwrap();
        assert!(
            out[..plan].contains("- now · settled"),
            "breadcrumb landed under Timeline, not at EOF:\n{out}"
        );
    }

    /// Second half of the same defect, for a file the app did not write the
    /// header of (a hand-edited dossier, or one from before the fix): the
    /// anchor scan starts at the SENTINEL with fresh fence state, so a fence
    /// opened in quoted text above it cannot reach past it.
    #[test]
    fn the_anchor_scan_starts_fresh_so_a_fence_above_it_is_irrelevant() {
        let file = format!(
            "# F\n\n## Context\n\n```text\npanic!\n\n{TIMELINE_HEADING}\n{TIMELINE_SENTINEL}\n\n\
             - a\n\n## Plan — x\n\ncontent\n"
        );
        let out = insert_under_timeline(&file, "- b");
        let plan = out.find("## Plan").unwrap();
        assert!(
            out[..plan].contains("- b"),
            "the unclosed fence above the sentinel swallowed the scan:\n{out}"
        );
    }

    #[test]
    fn demote_headings_closes_a_fence_the_quoted_text_left_open() {
        assert_eq!(demote_headings("```rs\nfn x() {}"), "```rs\nfn x() {}\n```");
        assert_eq!(demote_headings("~~~~\nstuff"), "~~~~\nstuff\n~~~~");
        // A balanced block is left exactly as it was.
        assert_eq!(demote_headings("```\nok\n```"), "```\nok\n```");
        assert_eq!(demote_headings("no fences here"), "no fences here");
    }

    #[test]
    fn demote_headings_only_touches_line_initial_hashes() {
        assert_eq!(demote_headings("## a\ntext\n# b"), "### a\ntext\n## b");
        assert_eq!(demote_headings("no # mid-line"), "no # mid-line");
        assert_eq!(demote_headings("  ## indented"), "  ## indented");
    }

    #[test]
    fn inline_flattens_multiline_fields() {
        assert_eq!(inline("a\n## Timeline\nb"), "a ## Timeline b");
        assert_eq!(inline("  spaced  "), "spaced");
    }

    #[test]
    fn acceptance_items_and_titles_cannot_inject_a_heading() {
        let mut i = issue("X\n## Timeline\n- forged title");
        i.acceptance = vec!["works\n## Timeline\n- forged ac".into()];
        i.prd_section = Some("## Flow\n## Timeline".into());
        i.prd_path = Some("docs/prds/x.md".into());
        let header = backfilled_header(&i, None);

        // Exactly one line-initial `## Timeline` in the whole file: ours.
        let headings = header
            .lines()
            .filter(|l| l.trim_end() == TIMELINE_HEADING)
            .count();
        assert_eq!(headings, 1, "one real anchor only:\n{header}");
        // The forged text survives as readable content, flattened.
        assert!(
            header.contains("- works ## Timeline - forged ac"),
            "{header}"
        );
        assert!(
            header.contains("# X ## Timeline - forged title"),
            "{header}"
        );
    }

    // -- the block the card detail reads (E19-06) --------------------------

    /// The viewer used to text-match the last `## Timeline` section, which
    /// disagrees with the appender's anchor in both directions: a bare
    /// `## Timeline` line an agent writes in prose steals the block (card
    /// renders empty while appends keep landing in the real one), and a
    /// RETITLED heading — which the sentinel exists to survive — finds
    /// nothing at all.
    #[test]
    fn the_block_resolves_on_the_sentinel_exactly_like_the_append_anchor() {
        // A decoy heading BELOW the real block: the last-heading rule would
        // pick it, the sentinel does not.
        let file = format!(
            "# F\n\n{TIMELINE_HEADING}\n{TIMELINE_SENTINEL}\n\n- a · real\n\n\
             ## Plan — x\n\nThe convention:\n\n## Timeline\n\n- a · decoy\n"
        );
        let block = timeline_block(&file).unwrap();
        assert_eq!(block.body, "- a · real", "{block:?}");
        // …and an append lands in the same block the reader just returned.
        let out = insert_under_timeline(&file, "- b · new");
        assert!(out.find("- b · new").unwrap() < out.find("## Plan").unwrap());

        // Retitled heading: the sentinel still anchors, and the block
        // reports the heading it now carries.
        let retitled = format!("# F\n\n## History\n{TIMELINE_SENTINEL}\n\n- a · real\n");
        let block = timeline_block(&retitled).unwrap();
        assert_eq!(block.heading, "History");
        assert_eq!(block.body, "- a · real");
    }

    #[test]
    fn the_block_stops_at_the_first_agent_section_and_skips_the_sentinel() {
        let file = format!(
            "# F\n\n{TIMELINE_HEADING}\n{TIMELINE_SENTINEL}\n\n\
             - a\n- b\n\n## Plan — x\n\nreasoning\n"
        );
        let block = timeline_block(&file).unwrap();
        assert_eq!(block.heading, "Timeline");
        assert_eq!(block.body, "- a\n- b");
        assert!(!block.body.contains(TIMELINE_SENTINEL));
    }

    /// Same trust boundary the append path has: a fence opened in quoted
    /// card text above the sentinel must not swallow the block.
    #[test]
    fn a_fence_above_the_sentinel_does_not_swallow_the_block() {
        let file = format!(
            "# F\n\n## Context\n\n```text\npanic!\n\n{TIMELINE_HEADING}\n{TIMELINE_SENTINEL}\n\n\
             - a\n\n## Plan — x\n\ncontent\n"
        );
        let block = timeline_block(&file).unwrap();
        assert_eq!(block.body, "- a", "{block:?}");
    }

    #[test]
    fn a_file_with_no_timeline_has_no_block() {
        assert!(timeline_block("# F\n\n## Plan — x\n\nstuff\n").is_none());
        assert!(timeline_block("").is_none());
        // Pre-sentinel dossier: the heading is the fallback anchor.
        let legacy = "## Timeline\n\n- a\n";
        assert_eq!(timeline_block(legacy).unwrap().body, "- a");
    }

    #[test]
    fn stamp_formats_a_known_epoch() {
        // 2026-08-09T12:34:00Z
        assert_eq!(format_stamp(1_786_278_840), "2026-08-09 12:34");
        assert_eq!(format_stamp(0), "1970-01-01 00:00");
    }
}
