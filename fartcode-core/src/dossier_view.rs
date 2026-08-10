//! The card-detail READ of a dossier (E19-06, #75; handoff v3 §8f).
//!
//! `dossiers` writes the file and `dossier_index` turns it into ⌘K rows;
//! this module is the third reader — the one the card detail renders. All
//! three go through the SAME section scan (`dossiers::sections`), so the
//! file the appender writes into, the file search indexes, and the file the
//! card shows can never disagree about where a section begins.
//!
//! **Why this is not TypeScript.** The Timeline line format is written by
//! [`dossiers::timeline_line`], the section boundaries are decided by
//! [`dossiers::section_heading_lines`] (fence-aware, hardened twice in this
//! epic against card text that forges structure). A parser in the webview
//! would be a second definition of both, free to drift on exactly the
//! inputs that already broke us — a card body that opens a code fence, a
//! quoted `## Timeline`, a heading inside a fenced sample.
//!
//! **What it does NOT do:** decide which file to read. Resolving the
//! worktree copy vs. the landed copy — and proving the file is this card's
//! dossier rather than a stranger's document at the same path — is the
//! app's job (`fartcode_app_lib::dossier_index::dossier_source`).

use crate::dossiers;

/// One `## Timeline` breadcrumb, folded and ready to render (§8f).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// The stamp exactly as written in the file, e.g. `2026-08-09 12:34`.
    /// Rendered when [`Self::at`] could not be parsed.
    pub stamp: String,
    /// The stamp as RFC3339 UTC (`2026-08-09T12:34:00Z`), or `None` when it
    /// is not a stamp we wrote. Dossier stamps are UTC by construction
    /// ([`dossiers::now_stamp`]) and so is SQLite's `datetime('now')`, which
    /// fills the backfilled `created` line — but the webview parses a bare
    /// `YYYY-MM-DD HH:MM` as LOCAL time, so the zone is made explicit here
    /// rather than left to the renderer to get wrong.
    pub at: Option<String>,
    /// The breadcrumb text. A launch and its settle are folded into one
    /// `<Column> · <provider> · launched → settled · <dur>` line; anything
    /// else passes through as written.
    pub text: String,
    /// A launch with no settle after it: the step is still going, so the
    /// renderer appends `running · <elapsed>` derived from [`Self::at`] on a
    /// tick (DESIGN.md: never store an elapsed).
    pub running: bool,
}

/// One agent-written `## <Column> — <date>` section (§8f's inset card).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DossierSection {
    /// Heading text without the `## ` marker, e.g. `Plan — 2026-08-09`.
    pub heading: String,
    /// The section's prose, trimmed.
    pub body: String,
}

/// The agent's own words: every `## ` section that is not one of the four
/// the app writes.
///
/// The same line ADR-0038 item 2 draws ("app writes the skeleton; agents
/// write the substance") and the same filter the ⌘K indexer applies
/// (`dossier_index::documents_checked`), through the same
/// [`dossiers::is_app_section`] predicate — so the sections the card offers
/// to walk are exactly the sections ⌘K can find.
pub fn agent_sections(content: &str) -> Vec<DossierSection> {
    dossiers::sections(content)
        .into_iter()
        .filter(|s| !s.heading.is_empty() && !dossiers::is_app_section(&s.heading))
        .map(|s| DossierSection {
            heading: s.heading,
            body: s.body,
        })
        .collect()
}

/// The rendered `## Timeline` (§8f), launch/settle pairs folded.
///
/// An empty vec means the file has no Timeline section — not that the card
/// has no dossier. §8f's "a skipped append = timeline intact, no inset
/// section" is the other direction: this returns entries while
/// [`agent_sections`] returns none.
pub fn timeline(content: &str) -> Vec<TimelineEntry> {
    fold(&raw_entries(content))
}

/// `dossiers::TIMELINE_HEADING` minus its `## ` marker — the heading text
/// [`dossiers::sections`] yields for the app's own block. Derived rather
/// than spelled again, so the two can never drift.
fn timeline_heading() -> &'static str {
    dossiers::TIMELINE_HEADING.trim_start_matches('#').trim()
}

/// `(stamp, fact)` for every `- ` line of the Timeline block.
///
/// The LAST Timeline section wins, mirroring the append anchor's
/// `rposition` rule: a `## Timeline` quoted earlier in the document (in card
/// text the header copied) is not the app's block, and the breadcrumbs are
/// being written to the last one.
fn raw_entries(content: &str) -> Vec<(String, String)> {
    let Some(block) = dossiers::sections(content)
        .into_iter()
        .rev()
        .find(|s| s.heading == timeline_heading())
    else {
        return Vec::new();
    };
    block
        .body
        .lines()
        .filter_map(|line| {
            let entry = line.trim().strip_prefix("- ")?;
            match entry.split_once(SEP) {
                Some((stamp, fact)) => Some((stamp.trim().to_string(), fact.trim().to_string())),
                // A line without the separator is still a breadcrumb
                // somebody wrote; render it rather than swallow it.
                None => Some((String::new(), entry.trim().to_string())),
            }
        })
        .collect()
}

/// The separator [`dossiers::timeline_line`] joins a breadcrumb's fields
/// with.
const SEP: &str = " · ";

/// A parsed `<Column> · launched · <provider>[ · <model>]` fact.
struct Launch<'a> {
    column: &'a str,
    provider: &'a str,
}

fn parse_launch(fact: &str) -> Option<Launch<'_>> {
    let mut parts = fact.split(SEP);
    let column = parts.next()?;
    if parts.next()? != "launched" {
        return None;
    }
    let provider = parts.next()?;
    Some(Launch { column, provider })
}

/// The column of a `<Column> · settled` fact. Exact: a longer line that
/// merely starts the same way is a different fact.
fn parse_settle(fact: &str) -> Option<&str> {
    let mut parts = fact.split(SEP);
    let column = parts.next()?;
    (parts.next()? == "settled" && parts.next().is_none()).then_some(column)
}

/// Folds `launched` + its `settled` into one §8f line, and marks a launch
/// with no settle as running.
///
/// Pairing is by column, first settle after the launch — the shape a
/// re-entered step actually produces (launch A, settle A, launch A, settle
/// A). An interleaving that does not pair leaves both lines standing on
/// their own rather than inventing a match.
fn fold(entries: &[(String, String)]) -> Vec<TimelineEntry> {
    let mut consumed = vec![false; entries.len()];
    let mut out: Vec<TimelineEntry> = Vec::with_capacity(entries.len());

    for i in 0..entries.len() {
        if consumed[i] {
            continue;
        }
        let (stamp, fact) = &entries[i];
        let at = parse_stamp(stamp);
        let Some(launch) = parse_launch(fact) else {
            out.push(TimelineEntry {
                stamp: stamp.clone(),
                at: at.map(|(_, iso)| iso),
                text: fact.clone(),
                running: false,
            });
            continue;
        };
        let settle = (i + 1..entries.len())
            .find(|&j| !consumed[j] && parse_settle(&entries[j].1) == Some(launch.column));
        let (text, running) = match settle {
            Some(j) => {
                consumed[j] = true;
                let dur = match (at.as_ref(), parse_stamp(&entries[j].0)) {
                    (Some((from, _)), Some((to, _))) if to >= *from => {
                        Some(duration_label(to - from))
                    }
                    _ => None,
                };
                let mut text = format!(
                    "{} · {} · launched → settled",
                    launch.column, launch.provider
                );
                if let Some(dur) = dur {
                    text.push_str(SEP);
                    text.push_str(&dur);
                }
                (text, false)
            }
            // The renderer appends `running · <elapsed>` in --text-card.
            None => (format!("{} · {}", launch.column, launch.provider), true),
        };
        out.push(TimelineEntry {
            stamp: stamp.clone(),
            at: at.map(|(_, iso)| iso),
            text,
            running,
        });
    }
    out
}

/// `41m` / `2h 5m` / `3d` — coarse, because the stamps are minute-resolution.
fn duration_label(secs: i64) -> String {
    let minutes = secs / 60;
    if minutes < 1 {
        return "<1m".to_string();
    }
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let (hours, rem_min) = (minutes / 60, minutes % 60);
    if hours < 24 {
        return if rem_min == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {rem_min}m")
        };
    }
    let (days, rem_hours) = (hours / 24, hours % 24);
    if rem_hours == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {rem_hours}h")
    }
}

/// `YYYY-MM-DD[ HH:MM[:SS]]` (UTC) → `(unix seconds, RFC3339)`.
///
/// Two producers write these: [`dossiers::now_stamp`] (minutes) and
/// SQLite's `datetime('now')` on `issues.created_at` (seconds). Both are
/// UTC. Anything else — an agent hand-editing the file — parses to `None`
/// and renders as the literal text it is.
fn parse_stamp(stamp: &str) -> Option<(i64, String)> {
    let (date, time) = match stamp.split_once(' ') {
        Some((date, time)) => (date, time.trim()),
        None => (stamp, ""),
    };
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut clock = time.split(':');
    let hour: i64 = match clock.next().filter(|s| !s.is_empty()) {
        Some(h) => h.parse().ok()?,
        None => 0,
    };
    let minute: i64 = match clock.next() {
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    let second: i64 = match clock.next() {
        Some(s) => s.parse().ok()?,
        None => 0,
    };
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let unix = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some((
        unix,
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
    ))
}

/// Days since the Unix epoch (Howard Hinnant's algorithm — the inverse of
/// the civil-date arithmetic `dossiers::now_stamp` formats WITH, and the
/// reason neither needs a date crate).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic dossier: backfilled header, app-owned Timeline with a
    /// settled step and a running one, two agent sections.
    fn dossier() -> String {
        format!(
            "# Implement OAuth login\n\n\
             {marker} (ADR-0038). -->\n\n\
             ## Context\n\nWe need login.\n\n\
             ## References\n\n- card: `iss_1`\n\n\
             {timeline}\n{sentinel}\n\n\
             - 2026-08-06 09:00:00 · created · proposal · docs/prds/oauth.md\n\
             - 2026-08-07 10:00 · Plan · launched · fable · sonnet\n\
             - 2026-08-07 10:41 · Plan · settled\n\
             - 2026-08-09 12:00 · Implement · launched · claude\n\n\
             ## Plan — 2026-08-07\n\n\
             Chose PKCE over the implicit flow.\n\n\
             ## Implement — 2026-08-09\n\n\
             Token refresh lives in the interceptor.\n",
            marker = dossiers::DOSSIER_MARKER,
            timeline = dossiers::TIMELINE_HEADING,
            sentinel = dossiers::TIMELINE_SENTINEL,
        )
    }

    #[test]
    fn only_the_agent_written_sections_are_offered() {
        let sections = agent_sections(&dossier());
        let headings: Vec<&str> = sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(
            headings,
            vec!["Plan — 2026-08-07", "Implement — 2026-08-09"]
        );
        assert_eq!(sections[0].body, "Chose PKCE over the implicit flow.");
    }

    #[test]
    fn a_launch_and_its_settle_fold_into_one_line_with_a_duration() {
        let entries = timeline(&dossier());
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "created · proposal · docs/prds/oauth.md",
                "Plan · fable · launched → settled · 41m",
                "Implement · claude",
            ],
            "the settle line is folded, not listed twice"
        );
        assert_eq!(entries[0].at.as_deref(), Some("2026-08-06T09:00:00Z"));
        assert_eq!(entries[1].at.as_deref(), Some("2026-08-07T10:00:00Z"));
    }

    /// §8f's "current step `running · <elapsed>`": the launch with no
    /// settle after it, and only that one.
    #[test]
    fn an_unsettled_launch_is_the_running_step() {
        let running: Vec<TimelineEntry> = timeline(&dossier())
            .into_iter()
            .filter(|e| e.running)
            .collect();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].text, "Implement · claude");
        assert_eq!(
            running[0].at.as_deref(),
            Some("2026-08-09T12:00:00Z"),
            "the elapsed ticks off the launch stamp, in UTC"
        );
    }

    #[test]
    fn a_re_entered_step_pairs_each_launch_with_its_own_settle() {
        let file = "## Timeline\n\n\
                    - 2026-08-09 09:00 · Plan · launched · fable\n\
                    - 2026-08-09 09:30 · Plan · settled\n\
                    - 2026-08-09 11:00 · Plan · launched · fable\n\
                    - 2026-08-09 13:00 · Plan · settled\n";
        let texts: Vec<String> = timeline(file).into_iter().map(|e| e.text).collect();
        assert_eq!(
            texts,
            vec![
                "Plan · fable · launched → settled · 30m",
                "Plan · fable · launched → settled · 2h",
            ]
        );
    }

    #[test]
    fn a_settle_for_another_column_never_closes_the_launch() {
        let file = "## Timeline\n\n\
                    - 2026-08-09 09:00 · Plan · launched · fable\n\
                    - 2026-08-09 09:30 · Implement · settled\n";
        let entries = timeline(file);
        assert!(entries[0].running, "Plan is still going");
        assert_eq!(entries[0].text, "Plan · fable");
        assert_eq!(entries[1].text, "Implement · settled");
    }

    /// Everything that is not a launch/settle pair is a breadcrumb the app
    /// wrote for a reason — column moves, PR lifecycle, the birth line.
    #[test]
    fn other_breadcrumbs_pass_through_verbatim() {
        let file = "## Timeline\n\n\
                    - 2026-08-09 09:00 · column · Ready → In Progress\n\
                    - 2026-08-09 09:01 · pr merged · https://x/pull/1\n\
                    - 2026-08-09 09:02 · dossier created with the worktree · Plan\n";
        let texts: Vec<String> = timeline(file).into_iter().map(|e| e.text).collect();
        assert_eq!(
            texts,
            vec![
                "column · Ready → In Progress",
                "pr merged · https://x/pull/1",
                "dossier created with the worktree · Plan",
            ]
        );
    }

    /// The whole reason this is not a TypeScript parser: a fenced sample of
    /// the section format (the seeded skill's own docs quote one) must not
    /// read as document structure, and a `## Timeline` quoted in card text
    /// must not be mistaken for the app's block.
    #[test]
    fn fenced_samples_and_quoted_headings_are_not_structure() {
        let file = format!(
            "# F\n\n## Context\n\n### Timeline\n\n- 1999 · forged\n\n\
             {timeline}\n{sentinel}\n\n- 2026-08-09 09:00 · created · manual\n\n\
             ## Plan — 2026-08-09\n\n\
             The skill documents the format:\n\n```md\n## Implement — <date>\n```\n",
            timeline = dossiers::TIMELINE_HEADING,
            sentinel = dossiers::TIMELINE_SENTINEL,
        );
        let entries = timeline(&file);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].text, "created · manual");

        let sections = agent_sections(&file);
        assert_eq!(
            sections.len(),
            1,
            "the fenced sample is body, not a section"
        );
        assert_eq!(sections[0].heading, "Plan — 2026-08-09");
    }

    #[test]
    fn a_dossier_with_no_timeline_or_no_sections_renders_neither() {
        assert!(timeline("# F\n\n## Plan — x\n\nstuff\n").is_empty());
        assert!(agent_sections(&format!(
            "# F\n\n{}\n\n- 2026-08-09 09:00 · created · manual\n",
            dossiers::TIMELINE_HEADING
        ))
        .is_empty());
        assert!(timeline("").is_empty() && agent_sections("").is_empty());
    }

    #[test]
    fn an_unparseable_stamp_still_renders_its_line() {
        let file = "## Timeline\n\n- yesterday · Plan · settled\n";
        let entries = timeline(file);
        assert_eq!(entries[0].stamp, "yesterday");
        assert_eq!(entries[0].at, None);
        assert_eq!(entries[0].text, "Plan · settled");
    }

    #[test]
    fn stamps_parse_as_utc_both_widths() {
        assert_eq!(
            parse_stamp("2026-08-09 12:34"),
            Some((1_786_278_840, "2026-08-09T12:34:00Z".to_string()))
        );
        assert_eq!(
            parse_stamp("1970-01-01 00:00:00"),
            Some((0, "1970-01-01T00:00:00Z".to_string()))
        );
        assert_eq!(parse_stamp("2026-13-01 00:00"), None);
        assert_eq!(parse_stamp("not a stamp"), None);
    }

    #[test]
    fn durations_read_at_the_scale_they_measure() {
        assert_eq!(duration_label(30), "<1m");
        assert_eq!(duration_label(41 * 60), "41m");
        assert_eq!(duration_label(2 * 3_600), "2h");
        assert_eq!(duration_label(2 * 3_600 + 5 * 60), "2h 5m");
        assert_eq!(duration_label(3 * 86_400), "3d");
        assert_eq!(duration_label(3 * 86_400 + 3_600), "3d 1h");
    }
}
