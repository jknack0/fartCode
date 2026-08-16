//! GitHub REST client (E4-07, #47) — `reqwest` + token from the keyring
//! (PRD: no Octokit). Parsing is split out into `pub` functions so it is
//! unit-tested against recorded fixtures without a network.
//!
//! Pagination: every endpoint caps at `per_page=100` — Phase 1 review
//! surfaces, not an archive.
//!
//! Rate limits: 401 → [`Error::GithubAuth`]; 403/429 with an exhausted
//! quota → [`Error::GithubRateLimited`] (carrying `X-RateLimit-Reset`);
//! 404 → `Ok(None)` where absence is data.

use serde::de::DeserializeOwned;

use crate::github::models::{
    to_pr_dto, GhCheckRuns, GhCommit, GhFile, GhIssueComment, GhPullRequest, GhReviewComment, PrDto,
};
use crate::Error;

/// GitHub REST API root (overridable so tests/GHES can point elsewhere).
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// Extracts `(owner, repo)` from a GitHub remote URL (scp-style, ssh://,
/// https://). Non-GitHub hosts and malformed URLs → `None`.
pub fn parse_github_slug(remote_url: &str) -> Option<(String, String)> {
    let path = remote_url
        .strip_prefix("git@github.com:")
        .or_else(|| remote_url.strip_prefix("https://github.com/"))
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

pub struct GitHubClient {
    http: reqwest::Client,
    token: String,
    api_base: String,
}

impl GitHubClient {
    /// Builds a client with the standard timeout (15 s) and API base.
    pub fn new(token: String) -> Result<Self, Error> {
        Self::with_api_base(token, DEFAULT_API_BASE)
    }

    pub fn with_api_base(token: String, api_base: &str) -> Result<Self, Error> {
        if token.trim().is_empty() {
            return Err(Error::GithubAuth("no GitHub token configured".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| Error::Github(format!("http client: {e}")))?;
        Ok(Self {
            http,
            token,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    /// The full PR view for a head branch: `None` when the branch has no
    /// open (or recently closed) PR. This is everything the PR tab renders
    /// — one call site, five sections.
    pub async fn fetch_pr_by_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PrDto>, Error> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls?state=open&head={}:{}",
            self.api_base,
            urlencoding(owner),
            urlencoding(branch)
        );
        let Some(list) = self.get_json_opt::<Vec<GhPullRequest>>(&url).await? else {
            return Ok(None);
        };
        let Some(pr) = list.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(self.fetch_pr_payload(owner, repo, pr).await?))
    }

    /// Fetches the PR detail (for merge state) + files/commits/checks/
    /// comments for a list-row PR.
    async fn fetch_pr_payload(
        &self,
        owner: &str,
        repo: &str,
        list_row: GhPullRequest,
    ) -> Result<PrDto, Error> {
        let number = list_row.number;
        // The detail call carries mergeable_state; fall back to the list
        // row if it 404s mid-flight (PR merged/deleted between calls).
        let detail_url = format!("{}/repos/{owner}/{repo}/pulls/{number}", self.api_base);
        let pr = match self.get_json_opt::<GhPullRequest>(&detail_url).await? {
            Some(detail) => detail,
            None => list_row,
        };
        let head_sha = pr.head.sha.clone();

        let files_url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}/files?per_page=100",
            self.api_base
        );
        let commits_url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}/commits?per_page=100",
            self.api_base
        );
        let checks_url = format!(
            "{}/repos/{owner}/{repo}/commits/{head_sha}/check-runs?per_page=100",
            self.api_base
        );
        let issue_url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/comments?per_page=100",
            self.api_base
        );
        let review_url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}/comments?per_page=100",
            self.api_base
        );

        let files = self
            .get_json_opt::<Vec<GhFile>>(&files_url)
            .await?
            .unwrap_or_default();
        let commits = self
            .get_json_opt::<Vec<GhCommit>>(&commits_url)
            .await?
            .unwrap_or_default();
        let checks = self
            .get_json_opt::<GhCheckRuns>(&checks_url)
            .await?
            .map(|c| c.check_runs)
            .unwrap_or_default();
        let issue_comments = self
            .get_json_opt::<Vec<GhIssueComment>>(&issue_url)
            .await?
            .unwrap_or_default();
        let review_comments = self
            .get_json_opt::<Vec<GhReviewComment>>(&review_url)
            .await?
            .unwrap_or_default();

        Ok(to_pr_dto(
            pr,
            files,
            commits,
            checks,
            issue_comments,
            review_comments,
        ))
    }

    /// The repository's default branch — the base for created PRs (#132).
    pub async fn default_branch(&self, owner: &str, repo: &str) -> Result<String, Error> {
        #[derive(serde::Deserialize)]
        struct GhRepo {
            default_branch: String,
        }
        let url = format!("{}/repos/{owner}/{repo}", self.api_base);
        let info = self
            .get_json_opt::<GhRepo>(&url)
            .await?
            .ok_or_else(|| Error::Github(format!("repository {owner}/{repo} not found on GitHub")))?;
        Ok(info.default_branch)
    }

    /// Creates a pull request (#132 — the "Commit, push & open PR" row).
    /// Returns the created PR as the same DTO the sync engine stores
    /// (sub-resources empty; the next sync pass fills them).
    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PrDto, Error> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.api_base);
        let payload = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body,
        });
        let pr: GhPullRequest = self.post_json(&url, &payload).await?;
        Ok(to_pr_dto(pr, vec![], vec![], vec![], vec![], vec![]))
    }

    // -- transport ----------------------------------------------------------

    /// GET returning parsed JSON; 404 → `Ok(None)` (absence is data).
    async fn get_json_opt<T: DeserializeOwned>(&self, url: &str) -> Result<Option<T>, Error> {
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fartCode")
            .send()
            .await
            .map_err(|e| Error::Github(format!("request failed: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::GithubAuth(
                "GitHub rejected the token (401) — re-import with 'gh auth token'".into(),
            ));
        }
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            let remaining = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u32>().ok());
            if remaining == Some(0) || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let reset_at = response
                    .headers()
                    .get("x-ratelimit-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<i64>().ok());
                return Err(Error::GithubRateLimited { reset_at });
            }
            return Err(Error::Github(format!("GitHub API forbidden ({status})")));
        }
        if !status.is_success() {
            return Err(Error::Github(format!("GitHub API error: {status}")));
        }

        let body = response
            .text()
            .await
            .map_err(|e| Error::Github(format!("response body: {e}")))?;
        serde_json::from_str(&body)
            .map_err(|e| Error::Github(format!("invalid JSON from GitHub: {e}")))
            .map(Some)
    }

    /// POST with a JSON payload. Non-2xx surfaces GitHub's own error
    /// message when the body carries one (422 validation — "A pull request
    /// already exists", "No commits between ...", ...).
    async fn post_json<T: DeserializeOwned>(
        &self,
        url: &str,
        payload: &serde_json::Value,
    ) -> Result<T, Error> {
        let response = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fartCode")
            .json(payload)
            .send()
            .await
            .map_err(|e| Error::Github(format!("request failed: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::GithubAuth(
                "GitHub rejected the token (401) — re-import with 'gh auth token'".into(),
            ));
        }
        let body = response
            .text()
            .await
            .map_err(|e| Error::Github(format!("response body: {e}")))?;
        if !status.is_success() {
            return Err(Error::Github(match github_error_detail(&body) {
                Some(detail) => format!("GitHub refused the request ({status}): {detail}"),
                None => format!("GitHub API error: {status}"),
            }));
        }
        serde_json::from_str(&body)
            .map_err(|e| Error::Github(format!("invalid JSON from GitHub: {e}")))
    }
}

/// GitHub's human-readable error from an error body: `message` plus the
/// first `errors[].message` when present (422 validation payloads).
fn github_error_detail(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let message = value.get("message")?.as_str()?.to_string();
    let detail = value
        .get("errors")
        .and_then(|e| e.as_array())
        .and_then(|errors| {
            errors
                .iter()
                .find_map(|e| e.get("message").and_then(|m| m.as_str()))
        });
    Some(match detail {
        Some(detail) => format!("{message}: {detail}"),
        None => message,
    })
}

/// Percent-encodes the reserved characters GitHub rejects in query values
/// (branch names may contain `/` — which is legal unencoded, but `#`, `&`,
/// `+` and friends are not).
fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => out.push(c),
            _ => {
                let mut buf = [0u8; 4];
                for byte in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_parsing_handles_remote_shapes() {
        assert_eq!(
            parse_github_slug("git@github.com:jknack0/fartCode.git"),
            Some(("jknack0".into(), "fartCode".into()))
        );
        assert_eq!(
            parse_github_slug("https://github.com/jknack0/fartCode.git"),
            Some(("jknack0".into(), "fartCode".into()))
        );
        assert_eq!(
            parse_github_slug("ssh://git@github.com/jknack0/fartCode"),
            Some(("jknack0".into(), "fartCode".into()))
        );
        assert_eq!(parse_github_slug("git@gitlab.com:o/r.git"), None);
        assert_eq!(parse_github_slug("/local/path"), None);
        assert_eq!(parse_github_slug("git@github.com:only-owner"), None);
    }

    #[test]
    fn url_encoding_keeps_slashes_and_escapes_the_rest() {
        assert_eq!(urlencoding("feat/x-y.z~1"), "feat/x-y.z~1");
        assert_eq!(urlencoding("a b&c#d"), "a%20b%26c%23d");
    }

    #[test]
    fn github_error_detail_surfaces_message_and_first_error() {
        let body = r#"{"message":"Validation Failed","errors":[{"message":"A pull request already exists for o:b."}]}"#;
        assert_eq!(
            github_error_detail(body).as_deref(),
            Some("Validation Failed: A pull request already exists for o:b.")
        );
        assert_eq!(
            github_error_detail(r#"{"message":"Not Found"}"#).as_deref(),
            Some("Not Found")
        );
        assert_eq!(github_error_detail("not json"), None);
    }

    #[test]
    fn empty_token_rejected() {
        assert!(matches!(
            GitHubClient::with_api_base("  ".into(), DEFAULT_API_BASE),
            Err(Error::GithubAuth(_))
        ));
    }

    #[test]
    fn parses_recorded_pr_list_fixture() {
        let list: Vec<GhPullRequest> = serde_json::from_str(fixtures::PR_LIST_JSON).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].number, 42);
        assert_eq!(list[0].head.name, "feat/widget");
        assert_eq!(list[0].status(), crate::github::models::PrStatus::Open);
    }

    #[test]
    fn parses_recorded_check_runs_fixture() {
        let runs: GhCheckRuns = serde_json::from_str(fixtures::CHECK_RUNS_JSON).unwrap();
        assert_eq!(runs.check_runs.len(), 2);
        assert_eq!(runs.check_runs[0].conclusion.as_deref(), Some("failure"));
    }

    #[test]
    fn parses_recorded_files_fixture() {
        let files: Vec<GhFile> = serde_json::from_str(fixtures::FILES_JSON).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[1].patch.is_none()); // binary → elided
    }

    #[test]
    fn full_fixture_payload_maps_to_dto() {
        // The whole recorded payload set → one denormalized DTO, exactly as
        // fetch_pr_payload assembles it (minus HTTP).
        let detail: GhPullRequest = serde_json::from_str(fixtures::PR_DETAIL_JSON).unwrap();
        let files: Vec<GhFile> = serde_json::from_str(fixtures::FILES_JSON).unwrap();
        let commits: Vec<GhCommit> = serde_json::from_str(fixtures::COMMITS_JSON).unwrap();
        let checks: GhCheckRuns = serde_json::from_str(fixtures::CHECK_RUNS_JSON).unwrap();
        let issue: Vec<GhIssueComment> =
            serde_json::from_str(fixtures::ISSUE_COMMENTS_JSON).unwrap();
        let review: Vec<GhReviewComment> =
            serde_json::from_str(fixtures::REVIEW_COMMENTS_JSON).unwrap();

        let dto = to_pr_dto(detail, files, commits, checks.check_runs, issue, review);
        assert_eq!(dto.number, 42);
        assert_eq!(dto.mergeable_state.as_deref(), Some("clean"));
        assert_eq!(dto.review_decision.as_deref(), Some("approved"));
        assert_eq!(dto.files.len(), 2);
        assert_eq!(dto.commits.len(), 1);
        assert_eq!(dto.checks[0].name, "ci/test"); // failure sorts first
        assert_eq!(dto.checks[0].conclusion.as_deref(), Some("failure"));
        assert_eq!(dto.comments.len(), 2);
        // DTO serializes camelCase for the frontend.
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["headOid"], "h1");
        assert_eq!(json["changedFiles"], 2);
    }
}

/// Recorded GitHub REST fixtures (acceptance: "client unit-tested against
/// recorded fixtures"). Shapes are real API payloads, trimmed.
#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::json;

    /// Builds a PR wire object with defaults + overrides (model tests).
    pub fn pr_json(overrides: serde_json::Value) -> serde_json::Value {
        let mut base = json!({
            "number": 42,
            "title": "Add widget",
            "html_url": "https://github.com/o/r/pull/42",
            "state": "open",
            "draft": false,
            "user": {"login": "jknack0", "avatar_url": "https://a/v.png", "html_url": "https://github.com/jknack0"},
            "base": {"ref": "main", "sha": "b0"},
            "head": {"ref": "feat/widget", "sha": "h1"},
            "additions": 0, "deletions": 0, "changed_files": 0, "commits": 0,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
            "merged_at": null
        });
        if let (Some(base), Some(o)) = (base.as_object_mut(), overrides.as_object()) {
            for (k, v) in o {
                base.insert(k.clone(), v.clone());
            }
        }
        base
    }

    pub const PR_LIST_JSON: &str = r#"[{
        "number": 42, "title": "Add widget",
        "html_url": "https://github.com/o/r/pull/42",
        "state": "open", "draft": false,
        "user": {"login": "jknack0"},
        "base": {"ref": "main", "sha": "b0"},
        "head": {"ref": "feat/widget", "sha": "h1"},
        "additions": 10, "deletions": 2, "changed_files": 2, "commits": 1,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-02T00:00:00Z",
        "merged_at": null
    }]"#;

    pub const CHECK_RUNS_JSON: &str = r#"{"total_count": 2, "check_runs": [
        {"id": 901, "name": "ci/test", "status": "completed", "conclusion": "failure",
         "html_url": "https://github.com/o/r/runs/901",
         "started_at": "2026-01-02T00:00:00Z", "completed_at": "2026-01-02T00:05:00Z",
         "app": {"name": "GitHub Actions"}},
        {"id": 902, "name": "ci/fmt", "status": "completed", "conclusion": "success",
         "html_url": "https://github.com/o/r/runs/902",
         "started_at": "2026-01-02T00:00:00Z", "completed_at": "2026-01-02T00:01:00Z",
         "app": {"name": "GitHub Actions"}}
    ]}"#;

    pub const FILES_JSON: &str = r#"[
        {"filename": "src/widget.rs", "status": "modified", "additions": 9, "deletions": 2,
         "patch": "@@ -1,3 +1,10 @@"},
        {"filename": "assets/logo.png", "status": "added", "additions": 0, "deletions": 0,
         "patch": null}
    ]"#;

    pub const COMMITS_JSON: &str = r#"[
        {"sha": "h1", "html_url": "https://github.com/o/r/commit/h1",
         "author": {"login": "jknack0"},
         "commit": {"message": "Add widget\n\nLong body",
                    "author": {"name": "Jon", "date": "2026-01-01T12:00:00Z"}}}
    ]"#;

    pub const ISSUE_COMMENTS_JSON: &str = r#"[
        {"id": 7001, "body": "LGTM", "html_url": "https://github.com/o/r/pull/42#issuecomment-7001",
         "user": {"login": "reviewer"},
         "created_at": "2026-01-02T01:00:00Z", "updated_at": "2026-01-02T01:00:00Z"}
    ]"#;

    pub const REVIEW_COMMENTS_JSON: &str = r#"[
        {"id": 8001, "body": "nit: rename", "html_url": "https://github.com/o/r/pull/42#discussion-8001",
         "user": {"login": "reviewer"}, "path": "src/widget.rs", "line": 4,
         "created_at": "2026-01-02T02:00:00Z", "updated_at": "2026-01-02T02:00:00Z"}
    ]"#;

    pub const PR_DETAIL_JSON: &str = r#"{
        "number": 42, "title": "Add widget",
        "html_url": "https://github.com/o/r/pull/42",
        "state": "open", "draft": false,
        "user": {"login": "jknack0"},
        "base": {"ref": "main", "sha": "b0"},
        "head": {"ref": "feat/widget", "sha": "h1"},
        "additions": 10, "deletions": 2, "changed_files": 2, "commits": 1,
        "mergeable_state": "clean", "review_decision": "APPROVED",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-02T00:00:00Z",
        "merged_at": null
    }"#;
}
