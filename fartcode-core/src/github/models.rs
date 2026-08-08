//! GitHub REST client types (E4-07, #47).
//!
//! Two layers: private *wire* structs parse GitHub's snake_case JSON
//! verbatim, and the public *DTO* structs serialize camelCase straight to
//! the frontend. Keeping them separate means the parser is testable against
//! recorded fixtures without serde rename gymnastics, and the DTO shape is
//! free to evolve independently of GitHub's wire format.
//!
//! The DTO set doubles as the E4-09 storage payload (versioned JSON), so
//! every DTO derives `Deserialize` too.

use serde::{Deserialize, Serialize};

/// PR lifecycle (REST `state` + `merged_at` collapse into three values —
/// reference `PullRequestStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrStatus {
    Open,
    Closed,
    Merged,
}

impl PrStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrStatus::Open => "open",
            PrStatus::Closed => "closed",
            PrStatus::Merged => "merged",
        }
    }
}

/// GitHub user (author / comment poster).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrUserDto {
    pub login: String,
    pub avatar_url: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhUser {
    pub login: String,
    pub avatar_url: Option<String>,
    pub html_url: Option<String>,
}

impl From<GhUser> for PrUserDto {
    fn from(u: GhUser) -> Self {
        PrUserDto {
            login: u.login,
            avatar_url: u.avatar_url,
            url: u.html_url,
        }
    }
}

/// One changed file (REST `pulls/{n}/files`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFileDto {
    pub filename: String,
    /// `added` | `removed` | `modified` | `renamed` | … (GitHub vocabulary).
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    /// `true` when GitHub elided the patch (files >300 lines / binary).
    pub patch_elided: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhFile {
    pub filename: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub patch: Option<String>,
}

/// One commit (REST `pulls/{n}/commits`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommitDto {
    pub sha: String,
    /// First line of the commit message (the list rows are dense).
    pub subject: String,
    pub author: Option<PrUserDto>,
    pub author_name: Option<String>,
    pub date: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhCommit {
    pub sha: String,
    pub html_url: Option<String>,
    pub author: Option<GhUser>,
    pub commit: GhCommitMeta,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhCommitMeta {
    pub message: Option<String>,
    pub author: Option<GhCommitIdentity>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhCommitIdentity {
    pub name: Option<String>,
    pub date: Option<String>,
}

/// One CI check run (REST `commits/{sha}/check-runs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheckDto {
    pub id: String,
    pub name: String,
    /// `queued` | `in_progress` | `completed`.
    pub status: String,
    /// `success` | `failure` | `cancelled` | `skipped` | …; `None` while
    /// the run is still going.
    pub conclusion: Option<String>,
    pub url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub app_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhCheckRuns {
    #[serde(default)]
    pub check_runs: Vec<GhCheckRun>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhCheckRun {
    pub id: serde_json::Value,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub html_url: Option<String>,
    pub details_url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub app: Option<GhApp>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhApp {
    pub name: Option<String>,
}

/// Comment kind (reference `PullRequestCommentKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrCommentKind {
    /// PR conversation comment (issues API).
    Issue,
    /// Line-anchored review comment (pulls/{n}/comments).
    Review,
}

/// One comment — issue and review comments merged into one chronological
/// list for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommentDto {
    pub id: String,
    pub kind: PrCommentKind,
    pub body: String,
    pub url: Option<String>,
    pub author: Option<PrUserDto>,
    /// File/line anchor for review comments.
    pub path: Option<String>,
    pub line: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhIssueComment {
    pub id: serde_json::Value,
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub user: Option<GhUser>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhReviewComment {
    pub id: serde_json::Value,
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub user: Option<GhUser>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// The full denormalized PR view — one row renders the whole PR tab, and
/// E4-09 stores it verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDto {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub status: PrStatus,
    pub draft: bool,
    pub author: Option<PrUserDto>,
    pub base_ref: String,
    pub head_ref: String,
    pub head_oid: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commit_count: i64,
    /// GitHub REST `mergeable_state` (`clean`|`dirty`|`blocked`|`behind`|
    /// `unstable`|`unknown`|`has_hooks`) — `None` while GitHub computes it.
    pub mergeable_state: Option<String>,
    /// `open` | `approved` | `changes_requested` (only on REST detail).
    pub review_decision: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub files: Vec<PrFileDto>,
    pub commits: Vec<PrCommitDto>,
    pub checks: Vec<PrCheckDto>,
    pub comments: Vec<PrCommentDto>,
}

/// Wire type for `GET /repos/{owner}/{repo}/pulls` rows and
/// `GET /repos/{owner}/{repo}/pulls/{n}`.
#[derive(Debug, Deserialize)]
pub(crate) struct GhPullRequest {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    pub user: Option<GhUser>,
    pub base: GhRef,
    pub head: GhRef,
    #[serde(default)]
    pub additions: i64,
    #[serde(default)]
    pub deletions: i64,
    #[serde(default)]
    pub changed_files: i64,
    #[serde(default)]
    pub commits: i64,
    /// Detail-only; the list endpoint omits it.
    pub mergeable_state: Option<String>,
    /// REST review state on the detail payload (`APPROVED` etc.).
    pub review_decision: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub merged_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GhRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub sha: String,
}

impl GhPullRequest {
    /// Collapses `state` + `merged_at` into the three-value status.
    pub fn status(&self) -> PrStatus {
        if self.merged_at.is_some() {
            PrStatus::Merged
        } else if self.state == "closed" {
            PrStatus::Closed
        } else {
            PrStatus::Open
        }
    }
}

/// Maps a merged GitHub payload to the UI/storage DTO. Sub-collections are
/// sorted for render order: checks failed-first then by name (ticket:
/// "failed checks sort to top"), comments chronologically (merge is stable).
pub(crate) fn to_pr_dto(
    pr: GhPullRequest,
    files: Vec<GhFile>,
    commits: Vec<GhCommit>,
    checks: Vec<GhCheckRun>,
    issue_comments: Vec<GhIssueComment>,
    review_comments: Vec<GhReviewComment>,
) -> PrDto {
    let status = pr.status();
    let mut dto = PrDto {
        number: pr.number,
        title: pr.title,
        url: pr.html_url,
        status,
        draft: pr.draft,
        author: pr.user.map(Into::into),
        base_ref: pr.base.name,
        head_ref: pr.head.name,
        head_oid: pr.head.sha,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        commit_count: pr.commits,
        mergeable_state: pr.mergeable_state,
        review_decision: pr.review_decision.map(|d| d.to_lowercase()),
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        files: files
            .into_iter()
            .map(|f| PrFileDto {
                patch_elided: f.patch.is_none(),
                filename: f.filename,
                status: f.status,
                additions: f.additions,
                deletions: f.deletions,
            })
            .collect(),
        commits: commits
            .into_iter()
            .map(|c| {
                let subject = c
                    .commit
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let identity = c.commit.author;
                let author_name = identity.as_ref().and_then(|a| a.name.clone());
                let date = identity.and_then(|a| a.date);
                PrCommitDto {
                    sha: c.sha,
                    subject,
                    author: c.author.map(Into::into),
                    author_name,
                    date,
                    url: c.html_url,
                }
            })
            .collect(),
        checks: checks
            .into_iter()
            .map(|c| PrCheckDto {
                id: json_id_to_string(c.id),
                name: c.name,
                status: c.status,
                conclusion: c.conclusion,
                url: c.html_url.or(c.details_url),
                started_at: c.started_at,
                completed_at: c.completed_at,
                app_name: c.app.and_then(|a| a.name),
            })
            .collect(),
        comments: {
            let mut all: Vec<PrCommentDto> = issue_comments
                .into_iter()
                .map(|c| PrCommentDto {
                    id: json_id_to_string(c.id),
                    kind: PrCommentKind::Issue,
                    body: c.body.unwrap_or_default(),
                    url: c.html_url,
                    author: c.user.map(Into::into),
                    path: None,
                    line: None,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                })
                .chain(review_comments.into_iter().map(|c| PrCommentDto {
                    id: json_id_to_string(c.id),
                    kind: PrCommentKind::Review,
                    body: c.body.unwrap_or_default(),
                    url: c.html_url,
                    author: c.user.map(Into::into),
                    path: c.path,
                    line: c.line,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                }))
                .collect();
            all.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            all
        },
    };
    dto.checks.sort_by_key(check_sort_key);
    dto
}

/// Failed checks first, then pending, then everything else — each group by
/// name. Returns an `Ord` key.
fn check_sort_key(check: &PrCheckDto) -> (u8, String) {
    let bucket = match (check.status.as_str(), check.conclusion.as_deref()) {
        ("completed", Some("failure")) => 0,
        ("completed", Some("cancelled" | "timed_out" | "action_required" | "startup_failure")) => 1,
        ("completed", _) => 3,
        _ => 2, // queued / in_progress between failures and passes
    };
    (bucket, check.name.clone())
}

/// GitHub ids arrive as numbers (REST) — stringify for our string-keyed
/// world.
fn json_id_to_string(id: serde_json::Value) -> String {
    match id {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::client::fixtures;

    #[test]
    fn merged_pr_status_collapses_state_and_merged_at() {
        let pr: GhPullRequest = serde_json::from_value(fixtures::pr_json(
            serde_json::json!({"state":"closed","merged_at":"2026-01-01T00:00:00Z"}),
        ))
        .unwrap();
        assert_eq!(pr.status(), PrStatus::Merged);

        let pr: GhPullRequest = serde_json::from_value(fixtures::pr_json(
            serde_json::json!({"state":"closed","merged_at":null}),
        ))
        .unwrap();
        assert_eq!(pr.status(), PrStatus::Closed);

        let pr: GhPullRequest =
            serde_json::from_value(fixtures::pr_json(serde_json::json!({}))).unwrap();
        assert_eq!(pr.status(), PrStatus::Open);
    }

    #[test]
    fn to_pr_dto_maps_and_sorts() {
        let pr: GhPullRequest = serde_json::from_value(fixtures::pr_json(serde_json::json!({
            "additions": 5, "deletions": 2, "changed_files": 3, "commits": 2,
            "mergeable_state": "clean"
        })))
        .unwrap();
        let files: Vec<GhFile> = serde_json::from_value(serde_json::json!([
            {"filename":"a.rs","status":"modified","additions":1,"deletions":1,"patch":"@@ -1 +1 @@"}
        ]))
        .unwrap();
        let commits: Vec<GhCommit> = serde_json::from_value(serde_json::json!([
            {"sha":"abc","html_url":"u","author":{"login":"jknack0"},
             "commit":{"message":"fix thing\n\nbody","author":{"name":"Jon","date":"2026-01-01T00:00:00Z"}}}
        ]))
        .unwrap();
        let checks: Vec<GhCheckRun> = serde_json::from_value(serde_json::json!([
            {"id":1,"name":"b-pass","status":"completed","conclusion":"success"},
            {"id":2,"name":"a-fail","status":"completed","conclusion":"failure"},
            {"id":3,"name":"pending","status":"in_progress","conclusion":null}
        ]))
        .unwrap();
        let issue: Vec<GhIssueComment> = serde_json::from_value(serde_json::json!([
            {"id":10,"body":"hello","html_url":"u","user":{"login":"jknack0"},
             "created_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:00:00Z"}
        ]))
        .unwrap();
        let review: Vec<GhReviewComment> = serde_json::from_value(serde_json::json!([
            {"id":11,"body":"nit","html_url":"u2","user":{"login":"rev"},
             "path":"a.rs","line":3,
             "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}
        ]))
        .unwrap();

        let dto = to_pr_dto(pr, files, commits, checks, issue, review);
        assert_eq!(dto.number, 42);
        assert_eq!(dto.status, PrStatus::Open);
        assert_eq!(dto.mergeable_state.as_deref(), Some("clean"));
        assert_eq!(dto.files.len(), 1);
        assert!(!dto.files[0].patch_elided);
        assert_eq!(dto.commits[0].subject, "fix thing");
        assert_eq!(dto.commits[0].author_name.as_deref(), Some("Jon"));
        // Failed checks sort first.
        let names: Vec<&str> = dto.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a-fail", "pending", "b-pass"]);
        // Comments merged chronologically (review first here).
        assert_eq!(dto.comments.len(), 2);
        assert_eq!(dto.comments[0].kind, PrCommentKind::Review);
        assert_eq!(dto.comments[0].path.as_deref(), Some("a.rs"));
        assert_eq!(dto.comments[1].kind, PrCommentKind::Issue);
    }

    #[test]
    fn dto_round_trips_through_json() {
        // E4-09 stores the DTO as versioned JSON — it must survive a
        // serialize/deserialize round trip verbatim.
        let pr: GhPullRequest =
            serde_json::from_value(fixtures::pr_json(serde_json::json!({}))).unwrap();
        let dto = to_pr_dto(pr, vec![], vec![], vec![], vec![], vec![]);
        let json = serde_json::to_string(&dto).unwrap();
        let back: PrDto = serde_json::from_str(&json).unwrap();
        assert_eq!(dto, back);
    }
}
