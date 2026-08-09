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
//! Two rules the whole feature rests on:
//!
//! - **Append-only, never rewrite.** The app owns exactly one section —
//!   `## Timeline` — and inserts lines at its end. Agent-written sections
//!   (`## <Column> — <date>`, ADR-0038 item 2, seeded by #71) are never
//!   read, reordered, or touched. Merge conflicts stay trivial because no
//!   two writers edit the same lines.
//! - **Memory, never a gate.** Every entry point here returns a `Result`
//!   the caller is expected to log and drop. A dossier that cannot be
//!   written must not stop an agent from running.

use std::path::{Path, PathBuf};

use crate::issues::Issue;
use crate::tasks::naming::generate_task_name;
use crate::Error;

/// Repo-relative directory every dossier lives in (ADR-0038 item 1).
pub const DOSSIER_DIR: &str = "docs/features";

/// The one heading the app writes under. Matched exactly (trailing
/// whitespace trimmed), so an agent section that merely starts with the
/// word is not mistaken for it.
pub const TIMELINE_HEADING: &str = "## Timeline";

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

/// One Timeline entry: `- <stamp> · <fact>`.
pub fn timeline_line(fact: &str) -> String {
    format!("- {} · {fact}", now_stamp())
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
    out.push_str(issue.title.trim());
    out.push_str("\n\n");
    out.push_str(
        "<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, \
         never rewrite existing ones. The app owns `## Timeline`; agents add \
         `## <Column> — <date>` sections below it. -->\n\n",
    );

    if let Some(body) = issue
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        out.push_str("## Context\n\n");
        out.push_str(body);
        out.push_str("\n\n");
    }

    if !issue.acceptance.is_empty() {
        out.push_str("## Acceptance\n\n");
        for item in &issue.acceptance {
            out.push_str("- ");
            out.push_str(item.trim());
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## References\n\n");
    out.push_str(&format!("- card: `{}`\n", issue.id));
    out.push_str(&format!("- source: {}\n", provenance(issue).label()));
    if let Some(prd) = issue.prd_path.as_deref().filter(|p| !p.is_empty()) {
        match issue.prd_section.as_deref().filter(|s| !s.is_empty()) {
            Some(section) => out.push_str(&format!("- PRD: `{prd}` — {section}\n")),
            None => out.push_str(&format!("- PRD: `{prd}`\n")),
        }
    }
    if let Some(url) = issue.external_ref.as_deref().filter(|u| !u.is_empty()) {
        out.push_str(&format!("- tracker: {url}\n"));
    }
    out.push('\n');

    out.push_str(TIMELINE_HEADING);
    out.push_str("\n\n");
    let created = issue
        .created_at
        .as_deref()
        .map(|c| c.trim().to_string())
        .unwrap_or_else(now_stamp);
    out.push_str(&format!(
        "- {created} · created · {}\n",
        provenance(issue).label()
    ));
    if !issue.blockers.is_empty() {
        let open: Vec<&str> = issue
            .blockers
            .iter()
            .filter(|b| !b.counts_as_done)
            .map(|b| b.title.as_str())
            .collect();
        if !open.is_empty() {
            out.push_str(&format!("- {created} · blocked by: {}\n", open.join(", ")));
        }
    }
    match column_name {
        Some(name) => out.push_str(&timeline_line(&format!(
            "dossier created with the worktree · {name}"
        ))),
        None => out.push_str(&timeline_line("dossier created with the worktree")),
    }
    out.push('\n');
    out
}

/// Creates the dossier at `<worktree>/<rel_path>` unless a file is already
/// there.
///
/// Returns `true` when this call wrote the file, `false` when it ADOPTED an
/// existing one. Adoption is the deliberate collision rule (ADR-0038 /
/// #70): a re-provisioned card keeps the dossier it already accumulated,
/// and a slug that collides with a landed feature's dossier appends to that
/// history rather than destroying it. Nothing here ever truncates.
pub fn create_dossier(worktree: &Path, rel_path: &str, header: &str) -> Result<bool, Error> {
    let target = worktree_file(worktree, rel_path);
    if target.exists() {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, header)?;
    Ok(true)
}

/// Appends one line to the dossier's `## Timeline` section.
///
/// `once_key`, when set, makes the append idempotent: if the file already
/// contains that substring the call is a no-op returning `false`. It exists
/// for facts that are true once but arrive repeatedly — `PrUpdated` fires
/// on every check/comment refresh, not only on open and merge.
///
/// Everything below the Timeline section is preserved byte for byte: the
/// insert point is the last non-blank line of the Timeline block, found by
/// scanning to the next `## ` heading. Agent sections are never parsed,
/// moved, or rewritten.
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
    let content = std::fs::read_to_string(&target)?;
    if let Some(key) = once_key {
        if content.contains(key) {
            return Ok(false);
        }
    }
    std::fs::write(&target, insert_under_timeline(&content, line))?;
    Ok(true)
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

/// Pure text surgery behind [`append_timeline`] — the whole append-safety
/// contract lives here, so it is unit-testable without a filesystem.
fn insert_under_timeline(content: &str, line: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let heading = lines.iter().position(|l| l.trim_end() == TIMELINE_HEADING);

    let Some(start) = heading else {
        // No Timeline section: an agent removed it, or the file was adopted
        // from a hand-written dossier. Start one at the end rather than
        // guessing where it belonged — appending is always safe.
        let mut out = content.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(TIMELINE_HEADING);
        out.push_str("\n\n");
        out.push_str(line);
        out.push('\n');
        return out;
    };

    // The Timeline block ends at the next `## ` heading (the first
    // agent-written section) or at EOF. `### ` subheadings do not end it.
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.starts_with("## "))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

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

    #[test]
    fn create_adopts_an_existing_file_instead_of_clobbering() {
        let tmp = tempfile::tempdir().unwrap();
        let rel = "docs/features/thing.md";
        assert!(create_dossier(tmp.path(), rel, "# fresh\n").unwrap());
        assert!(!create_dossier(tmp.path(), rel, "# SECOND\n").unwrap());
        let on_disk = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert_eq!(on_disk, "# fresh\n");
    }

    #[test]
    fn once_key_makes_an_append_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let rel = "docs/features/thing.md";
        create_dossier(tmp.path(), rel, "# f\n\n## Timeline\n\n- created\n").unwrap();
        let key = "pr merged · https://x/1";
        assert!(append_timeline(tmp.path(), rel, &timeline_line(key), Some(key)).unwrap());
        assert!(!append_timeline(tmp.path(), rel, &timeline_line(key), Some(key)).unwrap());
        let on_disk = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert_eq!(on_disk.matches(key).count(), 1);
    }

    #[test]
    fn append_to_a_missing_dossier_errors_rather_than_creating_one() {
        let tmp = tempfile::tempdir().unwrap();
        let err = append_timeline(tmp.path(), "docs/features/gone.md", "- x", None);
        assert!(err.is_err(), "a torn-down worktree records nothing");
    }

    #[test]
    fn stamp_formats_a_known_epoch() {
        // 2026-08-09T12:34:00Z
        assert_eq!(format_stamp(1_786_278_840), "2026-08-09 12:34");
        assert_eq!(format_stamp(0), "1970-01-01 00:00");
    }
}
