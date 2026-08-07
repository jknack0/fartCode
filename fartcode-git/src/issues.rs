//! GitHub issue listing (E17 dogfood): `gh issue list` over the project
//! checkout. The reference drives GitHub through the `gh` CLI (E8 lands the
//! token-backed engine); this is the minimal read path the project page
//! needs today — `find_on_path("gh")`, one JSON query, one parse.

use std::path::Path;
use std::process::Command;

use fartcode_core::pty::launcher::find_on_path;
use fartcode_core::Error;
use serde::{Deserialize, Serialize};

/// One GitHub issue for the project page (camelCase DTO straight over the
/// wire). Carries every field the native import maps: the full body,
/// labels, assignees, milestone, and timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub created_at: Option<String>,
}

/// Raw `gh issue list` row — labels/assignees/milestone arrive as objects.
#[derive(Debug, Deserialize)]
struct GhIssueRow {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(default)]
    assignees: Vec<GhUser>,
    #[serde(default)]
    milestone: Option<GhMilestone>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhMilestone {
    title: String,
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
            body: row.body,
            labels: row.labels.into_iter().map(|l| l.name).collect(),
            assignees: row.assignees.into_iter().map(|u| u.login).collect(),
            milestone: row.milestone.map(|m| m.title),
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
            "number,title,url,body,labels,assignees,milestone,createdAt",
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

/// A GitHub issue mapped onto native issue fields (E17 dogfood import).
/// Title carries the `#N` prefix; checkbox lines are pulled into
/// acceptance criteria; labels/assignees/milestone become a metadata
/// header in the body — nothing from the source issue is lost.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedIssue {
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Vec<String>,
}

/// Maps a GitHub issue onto native issue fields. Checkbox lines
/// (`- [ ]` / `- [x]`) become acceptance criteria; labels, assignees and
/// the milestone become a metadata header; everything else survives as
/// body content.
pub fn map_issue_fields(gh: &GitHubIssue) -> MappedIssue {
    let mut acceptance: Vec<String> = Vec::new();
    let mut body_lines: Vec<&str> = Vec::new();
    for line in gh.body.as_deref().unwrap_or_default().lines() {
        let trimmed = line.trim_start();
        let is_checkbox = trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ");
        if is_checkbox {
            acceptance.push(trimmed[6..].trim().to_string());
        } else {
            body_lines.push(line);
        }
    }
    let mut sections: Vec<String> = Vec::new();
    if !gh.labels.is_empty() {
        sections.push(format!("Labels: {}", gh.labels.join(", ")));
    }
    if !gh.assignees.is_empty() {
        sections.push(format!("Assignees: {}", gh.assignees.join(", ")));
    }
    if let Some(m) = &gh.milestone {
        sections.push(format!("Milestone: {m}"));
    }
    let remnant = body_lines.join("\n").trim().to_string();
    let body = match (remnant.is_empty(), sections.is_empty()) {
        (false, false) => Some(format!("{}\n\n{}", sections.join(" · "), remnant)),
        (false, true) => Some(remnant),
        (true, false) => Some(sections.join(" · ")),
        (true, true) => None,
    };
    MappedIssue {
        title: format!("#{} {}", gh.number, gh.title),
        body,
        acceptance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_golden_gh_output() {
        let json = r#"[
            {"number": 12, "title": "Fix the flaky test",
             "url": "https://github.com/o/r/issues/12",
             "body": "It flakes on CI.",
             "labels": [{"name": "bug"}, {"name": "p1"}],
             "assignees": [{"login": "drfart"}],
             "milestone": {"title": "Phase 1"},
             "createdAt": "2026-08-01T10:00:00Z"},
            {"number": 13, "title": "No labels here",
             "url": "https://github.com/o/r/issues/13",
             "body": null,
             "labels": [],
             "assignees": [],
             "milestone": null,
             "createdAt": null}
        ]"#;
        let issues = parse_gh_issues(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 12);
        assert_eq!(issues[0].title, "Fix the flaky test");
        assert_eq!(issues[0].body.as_deref(), Some("It flakes on CI."));
        assert_eq!(issues[0].labels, vec!["bug", "p1"]);
        assert_eq!(issues[0].assignees, vec!["drfart"]);
        assert_eq!(issues[0].milestone.as_deref(), Some("Phase 1"));
        assert_eq!(
            issues[0].created_at.as_deref(),
            Some("2026-08-01T10:00:00Z")
        );
        assert!(issues[1].labels.is_empty());
        assert!(issues[1].created_at.is_none());
    }

    #[test]
    fn map_issue_fields_extracts_acceptance_and_body() {
        let gh = GitHubIssue {
            number: 42,
            title: "Add the feature".into(),
            url: "https://github.com/o/r/issues/42".into(),
            body: Some(
                "**Story:** do the thing\n\n- [ ] step one\n- [x] step two\n- not a checkbox"
                    .into(),
            ),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            created_at: None,
        };
        let mapped = map_issue_fields(&gh);
        assert_eq!(mapped.title, "#42 Add the feature");
        assert_eq!(
            mapped.body.as_deref(),
            Some("**Story:** do the thing\n\n- not a checkbox")
        );
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
