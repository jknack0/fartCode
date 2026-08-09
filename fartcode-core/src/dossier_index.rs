//! Dossier sections as ⌘K `feature` rows (E19-03, #72; ADR-0038 item 4).
//!
//! One `search_index` row per dossier SECTION — `item_type: "feature"`,
//! title = the section heading, keywords extracted from the section body.
//! New item types are rows, not schema (ADR-0038 item 4), so nothing here
//! touches the FTS table definition.
//!
//! This module is the pure half: text in, [`search::Document`]s out, plus
//! the three set operations the index needs (replace a card's sections,
//! forget a card, forget a project). Resolving WHICH file to read — the
//! worktree copy on settle, the main-branch copy on project pull — is the
//! app's job (`fartcode_app_lib::dossier_index`), because only the app can
//! reach workspaces and project roots.
//!
//! **What is indexed.** The agent's words. [`dossiers::sections`] splits
//! the file; [`dossiers::is_app_section`] drops the four the app itself
//! wrote — `Context`, `Acceptance`, `References` and `Timeline`. Machine
//! breadcrumbs are not search material, and the header sections are
//! copies of card fields the app can re-derive at any time; indexing them
//! would return three near-identical `feature` hits per card, all opening
//! the same card detail. (Spec note: ADR-0038 says "one per dossier
//! section" without qualifying it. Item 2's "app writes the skeleton;
//! agents write the substance" is the line drawn here, and #72's
//! acceptance — header + Timeline + two agent sections = two rows —
//! settles it the same way.)
//!
//! **Never a gate.** Same posture as the dossier writer: every entry point
//! returns a `Result` the caller logs and drops. A reindex must not fail a
//! settle or a pull.

use std::collections::HashSet;
use std::sync::Arc;

use crate::db::Db;
use crate::dossiers;
use crate::search::{self, Document};
use crate::Error;

/// The `item_type` every dossier-section row carries (ADR-0038 item 4).
pub const ITEM_TYPE: &str = "feature";

/// Separates the card id from the section key inside an `item_id`. Issue
/// ids are `iss_<uuid>` — `[a-z0-9_-]` — so `#` cannot occur in the left
/// half and the split is unambiguous in both directions.
const SEP: char = '#';

/// Cap on the `keywords` column, in bytes. A dossier section is prose, but
/// nothing stops an agent from pasting a 2 MB log into one; the index is
/// for finding the section, not for storing it.
const MAX_KEYWORD_BYTES: usize = 4_000;

/// The `item_id` prefix owning every row of one card's dossier — the group
/// key [`search::replace_group`] / [`search::delete_group`] operate on.
pub fn item_prefix(issue_id: &str) -> String {
    format!("{issue_id}{SEP}")
}

/// The `item_id` for one section: `<issue id>#<heading>`, with a `#<n>`
/// tail for the nth repeat of a heading in the same file.
///
/// **Why this is stable.** The heading is the section's identity, and a
/// dossier is append-only by construction (ADR-0038 item 2 — agents add
/// sections, they do not rewrite them). So:
///
/// - An agent EDITING a section in place — fixing its reasoning, adding a
///   rejected alternative — leaves the heading alone, so the id is
///   unchanged, so the deterministic FNV-1a rowid is unchanged, so the
///   reindex REPLACES the row instead of adding a second one. That is the
///   whole idempotence story, and it holds without storing any state
///   beside the file.
/// - RETITLING a section is, correctly, a different section: the new id
///   gets a row and the old one is pruned by the same
///   [`search::replace_group`] call, so ⌘K never shows the old heading.
/// - `ordinal` only moves if the ORDER of same-titled sections changes,
///   which append-only writing does not do. Two `## Implement —
///   2026-08-09` sections (one column entered twice in a day) therefore
///   keep stable, distinct ids rather than collapsing into one row.
///
/// Deliberately NOT derived from the dossier PATH: a card that steps aside
/// onto a disambiguated filename (`<slug>-<short id>.md`) must not orphan
/// every row it already had.
pub fn item_id(issue_id: &str, heading: &str, ordinal: usize) -> String {
    let heading = heading.trim();
    if ordinal == 0 {
        format!("{issue_id}{SEP}{heading}")
    } else {
        format!("{issue_id}{SEP}{heading}{SEP}{}", ordinal + 1)
    }
}

/// The card a `feature` row belongs to.
///
/// §8h renders a hit as `<Column> — <feature title>` with right-meta
/// `feature · #id` and opens the CARD DETAIL on Enter, so every row has to
/// resolve to an issue id without a second lookup. It does: the id is the
/// left half of `item_id`. Returns `None` for a row that is not one of
/// ours.
pub fn issue_id_of(item_id: &str) -> Option<&str> {
    item_id.split_once(SEP).map(|(issue_id, _)| issue_id)
}

/// The rows a dossier's text produces for one card.
///
/// Pure: no filesystem, no DB. `content` is the dossier file as read from
/// whichever copy the caller resolved.
pub fn documents(issue_id: &str, project_id: &str, content: &str) -> Vec<Document> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for section in dossiers::sections(content) {
        if section.heading.is_empty() || dossiers::is_app_section(&section.heading) {
            continue;
        }
        // Ordinal counts EARLIER indexed sections carrying this heading, so
        // the numbering a reindex computes depends on the file alone.
        let ordinal = seen.iter().filter(|h| **h == section.heading).count();
        seen.push(section.heading.clone());
        out.push(Document {
            item_id: item_id(issue_id, &section.heading, ordinal),
            project_id: Some(project_id.to_string()),
            // The dossier outlives the task (ADR-0038 item 4: "issue rows
            // persist after the task is gone"), and the hit opens the card
            // detail, not a workspace — a task id here would be a link
            // that dangles the moment the worktree is torn down.
            task_id: None,
            keywords: keywords(&section.heading, &section.body),
            title: section.heading,
        });
    }
    out
}

/// Replaces this card's `feature` rows with what `content` says — the
/// incremental-safe reindex.
///
/// Idempotent (the deterministic rowid makes each write a replace) and
/// self-pruning (a section deleted from the file loses its row). Returns
/// the number of rows the dossier now has.
pub fn reindex(
    db: &Arc<dyn Db>,
    issue_id: &str,
    project_id: &str,
    content: &str,
) -> Result<usize, Error> {
    let docs = documents(issue_id, project_id, content);
    search::replace_group(db, ITEM_TYPE, &item_prefix(issue_id), &docs)
}

/// Drops every `feature` row of one card — the card was deleted, or its
/// dossier left with an unmerged branch (ADR-0038 item 5). Orphan rows
/// would make ⌘K open a card detail for a dead issue.
pub fn forget_issue(db: &Arc<dyn Db>, issue_id: &str) -> Result<usize, Error> {
    search::delete_group(db, ITEM_TYPE, &item_prefix(issue_id))
}

/// Drops every `feature` row of one project. Project deletion cascades its
/// issues in SQL WITHOUT emitting a per-issue `IssueDeleted`, so the
/// per-card path above never runs for them.
pub fn forget_project(db: &Arc<dyn Db>, project_id: &str) -> Result<usize, Error> {
    search::delete_by_project(db, ITEM_TYPE, project_id)
}

/// Heading + body, deduped and capped, for the `keywords` column.
///
/// The trigram tokenizer already matches substrings, so this needs no
/// stemming — it needs to be SMALL. Words are lowercased (the tokenizer is
/// case-insensitive, so a second casing is pure bloat), stripped of
/// leading/trailing punctuation, and deduped, which collapses the
/// repetition prose is full of.
fn keywords(heading: &str, body: &str) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = String::new();
    for word in heading.split_whitespace().chain(body.split_whitespace()) {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        let word = word.to_lowercase();
        if !seen.insert(word.clone()) {
            continue;
        }
        if out.len() + word.len() + 1 > MAX_KEYWORD_BYTES {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    const ISSUE: &str = "iss_1111-2222";
    const PROJECT: &str = "p1";

    fn db() -> Arc<dyn Db> {
        SqliteDb::init_in_memory().unwrap()
    }

    /// A realistic dossier: backfilled header, app-owned Timeline, two
    /// agent-written sections.
    fn dossier() -> String {
        format!(
            "# Implement OAuth login\n\n\
             {marker} (ADR-0038). -->\n\n\
             ## Context\n\nWe need login.\n\n\
             ## Acceptance\n\n- it works\n\n\
             ## References\n\n- card: `{ISSUE}`\n\n\
             {timeline}\n{sentinel}\n\n\
             - 2026-08-09 10:00 · created · manual\n\n\
             ## Plan — 2026-08-09\n\n\
             Chose PKCE over the implicit flow; rejected a session cookie \
             because the mobile client cannot hold one.\n\n\
             ## Implement — 2026-08-09\n\n\
             Token refresh lives in the interceptor.\n",
            marker = dossiers::DOSSIER_MARKER,
            timeline = dossiers::TIMELINE_HEADING,
            sentinel = dossiers::TIMELINE_SENTINEL,
        )
    }

    #[test]
    fn only_the_agent_written_sections_become_rows() {
        let docs = documents(ISSUE, PROJECT, &dossier());
        let titles: Vec<&str> = docs.iter().map(|d| d.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Plan — 2026-08-09", "Implement — 2026-08-09"],
            "header + Timeline are the app's skeleton, not search material"
        );
        assert!(docs
            .iter()
            .all(|d| d.project_id.as_deref() == Some(PROJECT)));
        assert!(docs.iter().all(|d| d.task_id.is_none()));
        // Keywords come from the BODY, not just the heading.
        assert!(docs[0].keywords.contains("pkce"), "{}", docs[0].keywords);
        assert!(docs[0].keywords.contains("rejected"));
    }

    #[test]
    fn every_row_resolves_to_its_card() {
        for doc in documents(ISSUE, PROJECT, &dossier()) {
            assert_eq!(
                issue_id_of(&doc.item_id),
                Some(ISSUE),
                "§8h opens the card detail on Enter: {}",
                doc.item_id
            );
        }
    }

    #[test]
    fn a_file_with_no_sections_indexes_nothing() {
        assert!(documents(ISSUE, PROJECT, "# Just a title\n\nprose\n").is_empty());
        assert!(documents(ISSUE, PROJECT, "").is_empty());
    }

    // -- hostile input ----------------------------------------------------

    #[test]
    fn a_heading_inside_a_fenced_block_is_not_a_section() {
        let file = "## Plan — 2026-08-09\n\n\
                    The seeded skill documents the format:\n\n\
                    ```md\n## Implement — <date>\n\ndecisions here\n```\n\n\
                    ~~~\n## Review — <date>\n~~~\n\n\
                    Still the plan.\n";
        let docs = documents(ISSUE, PROJECT, file);
        assert_eq!(docs.len(), 1, "one section: {:?}", titles(&docs));
        assert_eq!(docs[0].title, "Plan — 2026-08-09");
        assert!(
            docs[0].keywords.contains("implement"),
            "the sample is still body text"
        );
    }

    #[test]
    fn subheadings_do_not_split_a_section() {
        let file = "## Plan — 2026-08-09\n\n### Tradeoffs\n\nA over B.\n\n### Rejected\n\nC.\n";
        let docs = documents(ISSUE, PROJECT, file);
        assert_eq!(docs.len(), 1, "{:?}", titles(&docs));
        assert!(docs[0].keywords.contains("tradeoffs"));
        assert!(docs[0].keywords.contains("rejected"));
    }

    #[test]
    fn crlf_and_trailing_whitespace_parse_the_same_as_lf() {
        let lf = "## Plan — 2026-08-09\n\nChose X.\n";
        let crlf = "## Plan — 2026-08-09  \r\n\r\nChose X.\r\n";
        let a = documents(ISSUE, PROJECT, lf);
        let b = documents(ISSUE, PROJECT, crlf);
        assert_eq!(a, b, "line endings are not content");
        assert_eq!(a[0].title, "Plan — 2026-08-09");
    }

    #[test]
    fn a_repeated_heading_keeps_both_sections() {
        let file = "## Implement — 2026-08-09\n\nfirst pass\n\n\
                    ## Implement — 2026-08-09\n\nsecond pass\n";
        let docs = documents(ISSUE, PROJECT, file);
        assert_eq!(docs.len(), 2, "neither section is swallowed");
        assert_ne!(docs[0].item_id, docs[1].item_id);
        assert!(docs[1].item_id.ends_with("#2"));
    }

    // -- the index --------------------------------------------------------

    #[test]
    fn reindex_is_idempotent_and_search_finds_the_sections() {
        let db = db();
        assert_eq!(reindex(&db, ISSUE, PROJECT, &dossier()).unwrap(), 2);
        assert_eq!(count(&db), 2);

        // Same file twice: the deterministic rowid replaces, never appends.
        reindex(&db, ISSUE, PROJECT, &dossier()).unwrap();
        assert_eq!(count(&db), 2, "no duplicate rows");

        let hits = search::query(&db, "PKCE", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].item_type, ITEM_TYPE);
        assert_eq!(hits[0].title, "Plan — 2026-08-09");
        assert_eq!(hits[0].project_id.as_deref(), Some(PROJECT));
        assert_eq!(issue_id_of(&hits[0].item_id), Some(ISSUE));
    }

    #[test]
    fn editing_a_section_in_place_updates_its_row_rather_than_adding_one() {
        let db = db();
        reindex(&db, ISSUE, PROJECT, &dossier()).unwrap();
        let edited = dossier().replace("Chose PKCE", "Chose device-code flow");
        reindex(&db, ISSUE, PROJECT, &edited).unwrap();

        assert_eq!(
            count(&db),
            2,
            "the heading did not change, so neither did the id"
        );
        assert_eq!(search::query(&db, "device-code", 10).unwrap().len(), 1);
        assert!(
            search::query(&db, "PKCE", 10).unwrap().is_empty(),
            "the old body is gone, not shadowed"
        );
    }

    #[test]
    fn a_deleted_section_loses_its_row_on_reindex() {
        let db = db();
        reindex(&db, ISSUE, PROJECT, &dossier()).unwrap();
        let shortened = dossier()
            .split("## Implement — 2026-08-09")
            .next()
            .unwrap()
            .to_string();
        assert_eq!(reindex(&db, ISSUE, PROJECT, &shortened).unwrap(), 1);
        assert_eq!(count(&db), 1);
        assert!(
            search::query(&db, "interceptor", 10).unwrap().is_empty(),
            "a section that no longer exists must not be findable"
        );
    }

    #[test]
    fn retitling_a_section_replaces_the_row_instead_of_doubling_it() {
        let db = db();
        reindex(&db, ISSUE, PROJECT, &dossier()).unwrap();
        let retitled = dossier().replace("## Plan — 2026-08-09", "## Grill — 2026-08-09");
        reindex(&db, ISSUE, PROJECT, &retitled).unwrap();
        assert_eq!(count(&db), 2);
        let hits = search::query(&db, "PKCE", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Grill — 2026-08-09", "old heading pruned");
    }

    #[test]
    fn forgetting_a_card_or_project_leaves_no_orphans() {
        let db = db();
        reindex(&db, ISSUE, PROJECT, &dossier()).unwrap();
        reindex(&db, "iss_other", PROJECT, &dossier()).unwrap();
        reindex(&db, "iss_elsewhere", "p2", &dossier()).unwrap();
        assert_eq!(count(&db), 6);

        assert_eq!(forget_issue(&db, ISSUE).unwrap(), 2);
        assert_eq!(count(&db), 4, "only this card's rows went");

        assert_eq!(forget_project(&db, PROJECT).unwrap(), 2);
        assert_eq!(count(&db), 2, "p2 untouched");
        let survivors = search::query(&db, "PKCE", 10).unwrap();
        assert_eq!(survivors.len(), 1);
        assert_eq!(issue_id_of(&survivors[0].item_id), Some("iss_elsewhere"));
    }

    /// Issue ids contain `_`, a `LIKE` wildcard — a prefix match written as
    /// `LIKE 'iss_1%'` would also delete `issX1…`'s rows.
    #[test]
    fn the_prefix_match_is_not_a_like_pattern() {
        let db = db();
        reindex(&db, "iss_1", PROJECT, "## A\n\nalpha\n").unwrap();
        reindex(&db, "issX1", PROJECT, "## B\n\nbravo\n").unwrap();
        assert_eq!(forget_issue(&db, "iss_1").unwrap(), 1);
        assert_eq!(search::query(&db, "bravo", 10).unwrap().len(), 1);
    }

    fn titles(docs: &[Document]) -> Vec<&str> {
        docs.iter().map(|d| d.title.as_str()).collect()
    }

    fn count(db: &Arc<dyn Db>) -> i64 {
        db.conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM search_index WHERE item_type = ?1",
                [ITEM_TYPE],
                |row| row.get(0),
            )
            .unwrap()
    }
}
