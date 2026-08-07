//! GitHub issue listing (E17 dogfood): `gh issue list` over the project
//! checkout. The reference drives GitHub through the `gh` CLI (E8 lands the
//! token-backed engine); this is the minimal read path the project page
//! needs today — `find_on_path("gh")`, one JSON query, one parse.

use std::path::Path;
use std::process::Command;

use ade_core::pty::launcher::find_on_path;
use ade_core::Error;
use serde::{Deserialize, Serialize};

/// One GitHub issue for the project page (camelCase DTO straight over the
/// wire).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub created_at: Option<String>,
}

/// Raw `gh issue list` row — labels arrive as objects; we keep the names.
#[derive(Debug, Deserialize)]
struct GhIssueRow {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

/// Parses the `gh issue list --json` payload. Malformed JSON or rows are
/// structured errors, never panics.
pub fn parse_gh_issues(json: &str) -> Result<Vec<GitHubIssue>, Error> {
    let rows: Vec<GhIssueRow> = serde_json::from_str(json)
        .map_err(|e| Error::Git(format!("gh issue list returned invalid JSON: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|row| GitHubIssue {
            number: row.number,
            title: row.title,
            url: row.url,
            labels: row.labels.into_iter().map(|l| l.name).collect(),
            created_at: row.created_at,
        })
        .collect())
}

/// Open issues of the repo checked out at `repo` (capped at 50 — the
/// project page is a triage surface, not an archive). Errors name the
/// remedy: gh missing from PATH, or not authenticated / no repo access.
pub fn list_github_issues(repo: &Path) -> Result<Vec<GitHubIssue>, Error> {
    let gh = find_on_path("gh").ok_or_else(|| {
        Error::Git("GitHub issues need the gh CLI on PATH (brew install gh)".into())
    })?;
    let output = Command::new(&gh)
        .current_dir(repo)
        .args([
            "issue",
            "list",
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,url,labels,createdAt",
        ])
        .output()
        .map_err(|e| Error::Git(format!("failed to run gh: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Git(format!(
            "gh issue list failed — {} ",
            stderr.trim()
        )));
    }
    parse_gh_issues(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_golden_gh_output() {
        let json = r#"[
            {"number": 12, "title": "Fix the flaky test",
             "url": "https://github.com/o/r/issues/12",
             "labels": [{"name": "bug"}, {"name": "p1"}],
             "createdAt": "2026-08-01T10:00:00Z"},
            {"number": 13, "title": "No labels here",
             "url": "https://github.com/o/r/issues/13",
             "labels": [],
             "createdAt": null}
        ]"#;
        let issues = parse_gh_issues(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 12);
        assert_eq!(issues[0].title, "Fix the flaky test");
        assert_eq!(issues[0].labels, vec!["bug", "p1"]);
        assert_eq!(
            issues[0].created_at.as_deref(),
            Some("2026-08-01T10:00:00Z")
        );
        assert!(issues[1].labels.is_empty());
        assert!(issues[1].created_at.is_none());
    }

    #[test]
    fn parse_empty_and_malformed() {
        assert!(parse_gh_issues("[]").unwrap().is_empty());
        assert!(matches!(parse_gh_issues("nope"), Err(Error::Git(_))));
        assert!(matches!(
            parse_gh_issues(r#"[{"title": "missing number"}]"#),
            Err(Error::Git(_))
        ));
    }
}
