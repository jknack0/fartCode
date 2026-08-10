//! Feature-dossier reads for the UI (E19-06, #75; handoff v3 §8f + §8h).
//!
//! Two commands, both read-only:
//!
//! - [`dossier_read`] — the card detail's Dossier group: the path, the
//!   app-written Timeline, and the agent-written sections.
//! - [`dossier_feature_rows`] — what a ⌘K `feature` hit needs beyond its
//!   indexed heading: the card it belongs to, and whether that card's
//!   dossier has LANDED.
//!
//! **Nothing here parses markdown.** `fartcode_core::dossier_view` does,
//! through the same fence-aware section scan the appender and the ⌘K
//! indexer use. A parser in the webview would be a second definition of the
//! file format, free to drift on exactly the inputs that broke this epic
//! twice (a card body opening a code fence, a quoted `## Timeline`).
//!
//! **Nothing here trusts a path.** The file is resolved through
//! `crate::dossier_index::dossier_source`, which accepts a candidate only
//! when [`dossiers::inspect`] says it carries the dossier marker AND this
//! card's `- card:` line. `docs/features/` is a common hand-written
//! convention; a stranger's document at the card's slug path must never be
//! read into the card's detail sheet.
//!
//! **Off the main thread.** Both commands read files, so both are `async`
//! and hand the whole body to `spawn_blocking` via [`off_main_thread`]
//! (AGENTS.md § "Tauri commands and the main thread"; the DB guard is taken
//! and dropped inside the closure).

use std::collections::HashMap;
use std::sync::Arc;

use tauri::State;

use fartcode_core::dossier_index;
use fartcode_core::dossier_view::{self, DossierSection, TimelineEntry};
use fartcode_core::issues::Issue;

use crate::app::App;
use crate::commands::git::off_main_thread;

/// The card detail's Dossier group (§8f).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DossierDto {
    /// Repo-relative path — the `--info` link's LABEL at the group's right.
    /// It is what the file is called on the branch, so it is also what a
    /// reader would grep for.
    pub path: String,
    /// Absolute path of the copy that was actually read — the worktree's
    /// while one exists, the project checkout's once the feature landed.
    /// The link opens THIS; the label names the file.
    pub host_path: String,
    /// App-written breadcrumbs, launch/settle folded.
    pub timeline: Vec<TimelineEntry>,
    /// Agent-written sections, in file order. Empty is the ordinary
    /// "the agent skipped the append" case: §8f renders the timeline and
    /// no inset card — never a nag.
    pub sections: Vec<DossierSection>,
}

/// One ⌘K `feature` row's card (§8h).
///
/// The row's TITLE half comes from the indexed section heading and its
/// ROUTE from `search`'s own `issueId`; this fills in only what neither
/// carries — the feature's own title and its display ref.
///
/// **No `landed` field, deliberately.** §8h appends ` · landed` "once
/// merged", and the app cannot answer that yet: the only cheap signal is
/// whether the dossier sits in the project checkout, which is a
/// WORKING-TREE fact, not an ancestry one — it is equally true of a branch
/// someone has checked out, or a file that was never merged at all. A
/// correct answer is a committed-content query against the project's base
/// ref (`git cat-file -e <base>:<path>`), and the app has no base-ref
/// resolver; adding one, plus a subprocess per hit per keystroke, is its
/// own ticket. Rendering nothing beats rendering a guess.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureRowDto {
    /// Echoes the `search_index.item_id` asked about, so the caller can
    /// match rows back without re-parsing anything.
    pub item_id: String,
    /// The card the row belongs to — `dossier_index::issue_id_of`, not a
    /// second parse. (`search` resolves the same id for routing; this
    /// echoes it so a caller holding only the row can still tell.)
    pub issue_id: String,
    /// The card's title (the "feature title" half of §8h's row title).
    pub title: String,
    /// The card's tracker URL, for the `#id` the board already derives.
    pub external_ref: Option<String>,
}

/// The card's dossier, or `None` when it has none.
///
/// `None` covers every "no dossier" shape at once — consent declined, a
/// pre-E19 card, a `dossier_path` whose file left with an unmerged branch,
/// or a foreign document sitting at the path. §8f: the card then renders NO
/// dossier group at all, not an empty state.
#[tauri::command]
pub async fn dossier_read(
    app: State<'_, Arc<App>>,
    issue_id: String,
) -> Result<Option<DossierDto>, String> {
    let app = app.inner().clone();
    off_main_thread(move || Ok(read_dossier(&app, &issue_id))).await
}

/// [`dossier_read`]'s whole body. Public so the integration suite can drive
/// it against a REAL provisioned worktree and project checkout — the file
/// resolution is the part worth testing, and it cannot be reached through
/// the `#[tauri::command]` wrapper without an app instance.
pub fn read_dossier(app: &App, issue_id: &str) -> Option<DossierDto> {
    let issue = app.issues.get(issue_id).ok().flatten()?;
    let rel = dossier_rel(&issue)?;
    let path = crate::dossier_index::dossier_source(app, &issue, rel)?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| tracing::warn!(issue = issue_id, path = %path.display(), error = %e, "dossier unreadable"))
        .ok()?;
    Some(DossierDto {
        path: rel.to_string(),
        host_path: path.to_string_lossy().into_owned(),
        timeline: dossier_view::timeline(&content),
        sections: dossier_view::agent_sections(&content),
    })
}

/// Resolves ⌘K `feature` hits to their cards. Ids that are not feature rows
/// — or whose card is gone — are simply absent from the result.
///
/// **Grouped by card, not by row.** One dossier produces one `feature` row
/// per section, so a query matching three sections of one feature used to
/// mean three identical card lookups — per keystroke. The cards are
/// resolved once and fanned back out over the ids that asked for them.
#[tauri::command]
pub async fn dossier_feature_rows(
    app: State<'_, Arc<App>>,
    item_ids: Vec<String>,
) -> Result<Vec<FeatureRowDto>, String> {
    let app = app.inner().clone();
    off_main_thread(move || Ok(feature_rows(&app, &item_ids))).await
}

/// [`dossier_feature_rows`]'s whole body. Public for the same reason
/// [`read_dossier`] is: the integration suite drives it against a real App.
pub fn feature_rows(app: &App, item_ids: &[String]) -> Vec<FeatureRowDto> {
    let mut cards: HashMap<&str, Option<Issue>> = HashMap::new();
    let mut out = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        let Some(issue_id) = dossier_index::issue_id_of(item_id) else {
            continue;
        };
        let card = cards
            .entry(issue_id)
            .or_insert_with(|| app.issues.get(issue_id).ok().flatten());
        let Some(issue) = card else {
            continue;
        };
        out.push(FeatureRowDto {
            item_id: item_id.clone(),
            issue_id: issue.id.clone(),
            title: issue.title.clone(),
            external_ref: issue.external_ref.clone(),
        });
    }
    out
}

/// The card's recorded dossier path, blank-normalized.
fn dossier_rel(issue: &Issue) -> Option<&str> {
    issue
        .dossier_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::dossiers;

    fn issue(id: &str, dossier: Option<&str>) -> Issue {
        Issue {
            id: id.into(),
            project_id: "p1".into(),
            title: "Invite vetting".into(),
            body: None,
            acceptance: Vec::new(),
            lane: fartcode_core::issues::Lane::InProgress,
            position: 0,
            provider: None,
            model: None,
            prd_path: None,
            prd_section: None,
            dossier_path: dossier.map(str::to_string),
            linked_task_id: None,
            external_ref: None,
            column_id: None,
            blocked: false,
            blockers: Vec::new(),
            created_at: None,
            updated_at: None,
        }
    }

    fn dossier_file(issue_id: &str) -> String {
        format!(
            "# Invite vetting\n\n{marker} -->\n\n\
             ## References\n\n{card}\n\n\
             {timeline}\n{sentinel}\n\n\
             - 2026-08-07 10:00 · Plan · launched · fable\n\
             - 2026-08-07 10:41 · Plan · settled\n\n\
             ## Plan — 2026-08-07\n\nGate the send path.\n",
            marker = dossiers::DOSSIER_MARKER,
            card = dossiers::card_marker(issue_id),
            timeline = dossiers::TIMELINE_HEADING,
            sentinel = dossiers::TIMELINE_SENTINEL,
        )
    }

    /// The whole read, minus the App: what the command builds once
    /// `dossier_source` has handed it a file it proved is ours.
    fn view(content: &str) -> (Vec<TimelineEntry>, Vec<DossierSection>) {
        (
            dossier_view::timeline(content),
            dossier_view::agent_sections(content),
        )
    }

    #[test]
    fn our_dossier_parses_into_a_timeline_and_its_sections() {
        let (timeline, sections) = view(&dossier_file("iss_1"));
        assert_eq!(timeline.len(), 1, "{timeline:?}");
        assert_eq!(timeline[0].text, "Plan · fable · launched → settled · 41m");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "Plan — 2026-08-07");
    }

    /// The bug found twice in this epic: `docs/features/` is a common
    /// hand-written convention, so a file EXISTING at the card's dossier
    /// path is not evidence it is the card's dossier. The read command
    /// resolves through `inspect`, never the path — so a stranger's
    /// document (and another card's dossier) is refused at the same path.
    #[test]
    fn a_foreign_file_at_the_same_path_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("invite-vetting.md");

        const HUMAN_DOC: &str = "# Invite vetting\n\nOur design notes. Not a dossier.\n";
        std::fs::write(&path, HUMAN_DOC).unwrap();
        assert_eq!(
            dossiers::inspect(&path, "iss_1"),
            dossiers::Occupant::Foreign,
            "a human's document is never this card's dossier"
        );

        // Another card's dossier at the same path is refused too — one
        // feature's reasoning must not surface on another's card.
        std::fs::write(&path, dossier_file("iss_other")).unwrap();
        assert_eq!(
            dossiers::inspect(&path, "iss_1"),
            dossiers::Occupant::OtherDossier
        );

        // Only the card's own file is read.
        std::fs::write(&path, dossier_file("iss_1")).unwrap();
        assert_eq!(
            dossiers::inspect(&path, "iss_1"),
            dossiers::Occupant::OurDossier
        );
    }

    #[test]
    fn a_card_with_no_dossier_path_reads_nothing() {
        assert_eq!(dossier_rel(&issue("iss_1", None)), None);
        assert_eq!(dossier_rel(&issue("iss_1", Some("   "))), None);
        assert_eq!(
            dossier_rel(&issue("iss_1", Some("docs/features/x.md"))),
            Some("docs/features/x.md")
        );
    }

    /// §8h's row resolves to its card through the index's own accessor —
    /// no second parse of `item_id` lives here.
    #[test]
    fn a_feature_row_resolves_to_its_card() {
        assert_eq!(
            dossier_index::issue_id_of("iss_1#Plan — 2026-08-07"),
            Some("iss_1")
        );
        assert_eq!(dossier_index::issue_id_of("not-a-feature-row"), None);
    }
}
