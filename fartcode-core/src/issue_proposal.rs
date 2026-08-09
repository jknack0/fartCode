//! PM-chat proposal blocks (E17-04, #58; ARCHITECTURE.md §13, ADR-0032).
//!
//! The PM agent never writes issues directly. It emits a fenced
//! ` ```fartCode-proposal ` JSON block in its reply; the transcript renders an
//! approval card; Approve calls [`apply_proposal`], which composes
//! [`IssueStore`] into issue rows + blocked-by edges.
//!
//! Two invariants:
//! - **Parsing never panics and never partially validates.** Malformed
//!   JSON, missing titles, empty issue lists, and duplicate titles all
//!   return `Err` — the frontend renders bad blocks as plain text.
//! - **Apply is all-or-nothing.** Any failure (unknown `blockedBy` title,
//!   dependency cycle) deletes the issues already created by the call
//!   (compensating rollback) before returning the error, so a half-applied
//!   proposal never litters the board.

use serde::{Deserialize, Serialize};

use crate::issues::{Issue, IssueStore, NewIssue};
use crate::Error;

/// The PRD the breakdown came from (the agent wrote it to the repo with its
/// own file tools; issues reference it by path + optional section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalPrd {
    pub path: String,
    pub title: Option<String>,
}

/// One proposed issue. `blocked_by` holds TITLES — resolved at apply time
/// against this proposal's issues first, then existing project issues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalIssue {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The parsed `fartCode-proposal` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    #[serde(default)]
    pub prd: Option<ProposalPrd>,
    pub issues: Vec<ProposalIssue>,
}

/// Parses a proposal block's JSON payload. Every failure is a structured
/// `Err(Error::InvalidProposal)` — never a panic, never a partial value.
pub fn parse_proposal(json: &str) -> Result<Proposal, Error> {
    let proposal: Proposal = serde_json::from_str(json)
        .map_err(|e| Error::InvalidProposal(format!("invalid JSON: {e}")))?;
    if proposal.issues.is_empty() {
        return Err(Error::InvalidProposal("proposal has no issues".into()));
    }
    for (i, issue) in proposal.issues.iter().enumerate() {
        if issue.title.trim().is_empty() {
            return Err(Error::InvalidProposal(format!(
                "issue {} has an empty title",
                i + 1
            )));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for issue in &proposal.issues {
        if !seen.insert(issue.title.trim().to_string()) {
            return Err(Error::InvalidProposal(format!(
                "duplicate issue title {:?} — titles must be unique within a proposal (blockedBy resolution)",
                issue.title
            )));
        }
    }
    Ok(proposal)
}

/// Applies an approved proposal: creates every issue (backlog lane, PRD
/// reference attached), then wires `blockedBy` edges. Atomic — any failure
/// deletes the issues this call created before returning the error.
pub fn apply_proposal(
    store: &IssueStore,
    project_id: &str,
    proposal: &Proposal,
) -> Result<Vec<Issue>, Error> {
    let mut created: Vec<Issue> = Vec::new();

    let result = apply_inner(store, project_id, proposal, &mut created);
    if let Err(e) = result {
        // Compensating rollback (single-user app; deletes are best-effort).
        for issue in &created {
            let _ = store.delete(&issue.id);
        }
        return Err(e);
    }
    Ok(created)
}

fn apply_inner(
    store: &IssueStore,
    project_id: &str,
    proposal: &Proposal,
    created: &mut Vec<Issue>,
) -> Result<(), Error> {
    for issue in &proposal.issues {
        created.push(store.create(NewIssue {
            project_id: project_id.into(),
            title: issue.title.clone(),
            body: issue.body.clone(),
            acceptance: issue.acceptance.clone(),
            // E18-06: approved issues land on the project's is_landing
            // column (Backlog by default, wherever the flag has moved).
            lane: None,
            provider: issue.provider.clone(),
            model: issue.model.clone(),
            prd_path: proposal.prd.as_ref().map(|p| p.path.clone()),
            prd_section: None,
            external_ref: None,
        })?);
    }

    // Title → id within this proposal (wins over existing issues).
    let own_titles: std::collections::HashMap<String, String> = created
        .iter()
        .map(|i| (i.title.clone(), i.id.clone()))
        .collect();
    // Existing project issues, excluding the ones just created (they are
    // already in `own_titles`). Exact-title match; first in board order
    // wins when existing titles collide (accepted v1 ambiguity).
    let existing = store.list_for_project(project_id)?;
    let resolve = |title: &str| -> Option<String> {
        if let Some(id) = own_titles.get(title) {
            return Some(id.clone());
        }
        existing
            .iter()
            .find(|i| !own_titles.values().any(|id| id == &i.id) && i.title == title)
            .map(|i| i.id.clone())
    };

    for (i, issue) in proposal.issues.iter().enumerate() {
        for blocker_title in &issue.blocked_by {
            let blocker_id = resolve(blocker_title).ok_or_else(|| {
                Error::InvalidProposal(format!(
                    "issue {:?} is blockedBy unknown title {:?}",
                    issue.title, blocker_title
                ))
            })?;
            store.add_dependency(&created[i].id, &blocker_id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, SqliteDb};
    use crate::events::BroadcastEventBus;
    use crate::issues::Lane;
    use std::sync::Arc;

    fn fixture() -> Arc<IssueStore> {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj')",
                [],
            )
            .unwrap();
            // Real projects carry the seeded board; apply lands cards on
            // its is_landing column (E18-06).
            crate::issues::columns::seed_default_columns(&conn, "p1").unwrap();
        }
        Arc::new(IssueStore::new(db, bus))
    }

    /// The project's landing column (Backlog on the seeded board).
    fn landing_column(store: &IssueStore) -> crate::issues::columns::BoardColumn {
        crate::issues::columns::ColumnStore::new(store.db())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.is_landing)
            .unwrap()
    }

    const GOLDEN: &str = r#"{
        "prd": { "path": "docs/prds/oauth.md", "title": "OAuth 2.0 PKCE" },
        "issues": [
            { "title": "Token storage", "body": "encrypted at rest",
              "acceptance": ["round-trips a token"], "blockedBy": [],
              "provider": "claude", "model": "opus" },
            { "title": "Middleware", "blockedBy": ["Token storage"] }
        ]
    }"#;

    #[test]
    fn parse_golden_full_block() {
        let p = parse_proposal(GOLDEN).unwrap();
        assert_eq!(p.prd.as_ref().unwrap().path, "docs/prds/oauth.md");
        assert_eq!(p.issues.len(), 2);
        assert_eq!(p.issues[0].title, "Token storage");
        assert_eq!(p.issues[0].acceptance, vec!["round-trips a token"]);
        assert_eq!(p.issues[0].provider.as_deref(), Some("claude"));
        assert_eq!(p.issues[1].blocked_by, vec!["Token storage"]);
    }

    #[test]
    fn parse_minimal_and_unknown_fields_ignored() {
        let p = parse_proposal(
            r#"{"issues": [{"title": "Only title", "futureField": 42}], "meta": true}"#,
        )
        .unwrap();
        assert!(p.prd.is_none());
        assert_eq!(p.issues[0].title, "Only title");
        assert!(p.issues[0].acceptance.is_empty());
        assert!(p.issues[0].blocked_by.is_empty());
    }

    #[test]
    fn parse_failures_err_never_panic() {
        for bad in [
            "not json at all",
            "{}",
            r#"{"issues": []}"#,
            r#"{"issues": [{"title": "  "}]}"#,
            r#"{"issues": [{"title": "x"}, {"title": "x"}]}"#,
            r#"{"issues": [{"body": "no title"}]}"#,
        ] {
            assert!(
                matches!(parse_proposal(bad), Err(Error::InvalidProposal(_))),
                "expected InvalidProposal for {bad:?}"
            );
        }
    }

    #[test]
    fn apply_creates_issues_with_prd_and_edges() {
        let store = fixture();
        let p = parse_proposal(GOLDEN).unwrap();
        let created = apply_proposal(&store, "p1", &p).unwrap();
        assert_eq!(created.len(), 2);
        // E18-06: approved issues land on the LANDING column (Backlog on
        // the seeded board), not on a hardcoded lane.
        let landing = landing_column(&store);
        assert!(created
            .iter()
            .all(|i| i.column_id.as_deref() == Some(landing.id.as_str())));
        assert!(created.iter().all(|i| i.lane == Lane::Backlog));
        assert_eq!(created[0].prd_path.as_deref(), Some("docs/prds/oauth.md"));
        // Edge wired: Middleware blocked by Token storage (derived).
        let mw = store.get(&created[1].id).unwrap().unwrap();
        assert!(mw.blocked);
        assert_eq!(mw.blockers[0].title, "Token storage");
    }

    /// E18-06: moving the `is_landing` flag moves the proposal-apply
    /// entry path with it — no Backlog hardcode survives.
    #[test]
    fn apply_lands_on_a_moved_landing_column() {
        let store = fixture();
        let columns = crate::issues::columns::ColumnStore::new(store.db());
        let triage = columns
            .create(crate::issues::columns::NewColumn {
                project_id: "p1".into(),
                name: "Triage".into(),
                kind: crate::issues::columns::ColumnKind::Shelf,
                counts_as_done: false,
                is_landing: true, // moves the flag off Backlog
                on_enter: None,
                on_settle: None,
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap();
        assert_eq!(landing_column(&store).id, triage.id);

        let p = parse_proposal(GOLDEN).unwrap();
        let created = apply_proposal(&store, "p1", &p).unwrap();
        assert!(created
            .iter()
            .all(|i| i.column_id.as_deref() == Some(triage.id.as_str())));
        // Triage is non-seeded, so the lane cannot represent it: the
        // legacy lane falls back to Backlog (interim until E18-07).
        assert!(created.iter().all(|i| i.lane == Lane::Backlog));
    }

    #[test]
    fn apply_resolves_blocker_against_existing_issues() {
        let store = fixture();
        let existing = store
            .create(NewIssue {
                project_id: "p1".into(),
                title: "Existing schema work".into(),
                body: None,
                acceptance: vec![],
                lane: Some(Lane::InProgress),
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
            })
            .unwrap();
        let p = parse_proposal(
            r#"{"issues": [{"title": "Follow-up", "blockedBy": ["Existing schema work"]}]}"#,
        )
        .unwrap();
        let created = apply_proposal(&store, "p1", &p).unwrap();
        let follow_up = store.get(&created[0].id).unwrap().unwrap();
        assert_eq!(follow_up.blockers[0].id, existing.id);
        assert!(follow_up.blocked); // existing issue is in_progress
    }

    #[test]
    fn apply_unknown_blocker_rolls_back_everything() {
        let store = fixture();
        let p = parse_proposal(
            r#"{"issues": [{"title": "Fine issue"}, {"title": "Bad ref", "blockedBy": ["ghost"]}]}"#,
        )
        .unwrap();
        assert!(matches!(
            apply_proposal(&store, "p1", &p),
            Err(Error::InvalidProposal(_))
        ));
        // "Fine issue" was created before the failure — must be gone.
        assert!(store.list_for_project("p1").unwrap().is_empty());
    }

    #[test]
    fn apply_cycle_within_proposal_rolls_back() {
        let store = fixture();
        let p = parse_proposal(
            r#"{"issues": [{"title": "X", "blockedBy": ["Y"]}, {"title": "Y", "blockedBy": ["X"]}]}"#,
        )
        .unwrap();
        assert!(matches!(
            apply_proposal(&store, "p1", &p),
            Err(Error::IssueDependencyCycle { .. })
        ));
        assert!(store.list_for_project("p1").unwrap().is_empty());
    }
}
