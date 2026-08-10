//! Signal 4 — **time-to-land trend**: cycle time as the knowledge base
//! grows.
//!
//! ADR-0038 item 7 calls this "the headline chart, and the noisiest", and
//! says it is "never presented without the caveat". The Consequences
//! paragraph adds where it comes from: "time-to-land derives from Timeline
//! events" — the `## Timeline` breadcrumbs the app appends to every dossier
//! (`fartcode_core::dossiers`), which unlike a transcript are committed to
//! the branch and still readable a year later. Nothing is captured on a hot
//! path for this signal; it is read from the repository on demand.
//!
//! # The caveat is part of the value, structurally
//!
//! [`TimeToLand`]'s fields are private. The only accessor,
//! [`TimeToLand::read`], hands back the figures **and** the caveat in one
//! value; [`TimeToLand::summary`] renders them together; and the derived
//! `Serialize` writes `caveat` on every payload, for every variant,
//! including the ones with no number in them.
//!
//! ```compile_fail
//! # use fartcode_telemetry::time_to_land::TimeToLand;
//! let t = TimeToLand::from_cycles(Vec::new());
//! // The figures are not reachable on their own — this does not compile.
//! let _ = t.kind;
//! ```
//!
//! That is deliberate and it is aimed at a specific failure: #76 renders
//! the chart, and a rule living in someone else's comment does not survive
//! the trip. It has to be impossible to pick up the number and leave the
//! sentence behind.
//!
//! # Why it is the noisiest
//!
//! Cycle time moves for every reason a software project moves: team size,
//! review latency, holidays, one gnarly feature, a change to the pipeline
//! itself. Dossiers are one input among dozens and the sample is a handful
//! of features. So this module deliberately reports **two medians and their
//! sample size**, never a percentage improvement and never a fitted slope —
//! a single "23% faster" invites exactly the attribution the caveat
//! forbids.

use serde::Serialize;

/// The sentence ADR-0038 item 7 requires beside every rendering of this
/// signal.
pub const TREND_CAVEAT: &str =
    "Trend, not attribution — your pipeline changed this month too. Cycle time moves for \
     many reasons; this chart cannot tell you which one.";

/// One parsed `## Timeline` breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// Unix seconds, from the line's `YYYY-MM-DD HH:MM[:SS]` stamp (UTC —
    /// `fartcode_core::dossiers::format_stamp` writes UTC).
    pub at: i64,
    /// Everything after the ` · ` separator, verbatim.
    pub fact: String,
}

/// One feature's measured cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureCycle {
    pub created: i64,
    pub landed: i64,
}

impl FeatureCycle {
    pub fn hours(&self) -> f64 {
        (self.landed - self.created) as f64 / 3_600.0
    }
}

/// Parses the body of a dossier's `## Timeline` section.
///
/// Takes the section body rather than the whole file on purpose: the app
/// layer splits dossiers with `fartcode_core::dossiers::sections`, the one
/// parser that also decides where the appender writes. A second, private
/// notion of "where the Timeline is" is exactly the drift that parser
/// exists to prevent.
///
/// Lines that are not breadcrumbs (the sentinel comment, blank lines, an
/// agent's stray prose) are skipped rather than guessed at.
pub fn parse_timeline(section_body: &str) -> Vec<TimelineEvent> {
    section_body
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("- ")?;
            let (stamp, fact) = rest.split_once(" · ")?;
            Some(TimelineEvent {
                at: parse_stamp(stamp.trim())?,
                fact: fact.trim().to_string(),
            })
        })
        .collect()
}

/// The cycle a dossier's timeline describes, or `None` when it does not
/// describe one.
///
/// **`created` → `pr merged`, and nothing looser.** The `created`
/// breadcrumb is backfilled at dossier birth from the card's `created_at`,
/// so it is the moment the feature existed, not the moment an agent
/// touched it. `pr merged` is the only breadcrumb that means *landed*.
///
/// Deliberately NOT inferred from anything else: "the last event in the
/// file" would count an abandoned branch as a landing, and a move into a
/// `counts_as_done` column is not recoverable from the breadcrumb text
/// (the line records column names, which are user-renameable). A project
/// whose features land without a tracked pull request reports
/// [`TimeToLandKind::NoData`] rather than a number built on a guess.
pub fn cycle_of(events: &[TimelineEvent]) -> Option<FeatureCycle> {
    let created = events
        .iter()
        .find(|e| e.fact.starts_with("created"))
        .map(|e| e.at)?;
    let landed = events
        .iter()
        .find(|e| e.fact.starts_with("pr merged"))
        .map(|e| e.at)?;
    // A clock skew or a hand-edited stamp must not produce a negative
    // cycle that then drags a median below zero.
    (landed >= created).then_some(FeatureCycle { created, landed })
}

/// What the window's landed features add up to.
///
/// Not `Copy`: `Trend` carries the per-cycle series (#76's sparkline), and
/// a `Vec` cannot be `Copy`. Callers clone through [`TimeToLand::read`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimeToLandKind {
    /// No feature in this project has both a `created` and a `pr merged`
    /// breadcrumb. The ordinary state of a project that has not merged a
    /// feature since dossiers were switched on.
    NoData,
    /// Exactly one landed feature.
    ///
    /// **A point is not a trend.** There is nothing to compare it to, so
    /// the honest answer is the one cycle time with no direction attached —
    /// not "0% change", not an arrow, not a sparkline of length one.
    SinglePoint { hours: f64 },
    /// Two or more, split by landing order into an earlier and a later
    /// half. For two features that is one each; the medians are then just
    /// the two values, which the sample size makes visible.
    Trend {
        earlier_median_hours: f64,
        later_median_hours: f64,
        landed: u32,
        /// Per-cycle hours in **landing order** — the same order the two
        /// medians were split on. The dashboard's sparkline is drawn from
        /// this; `NoData` and `SinglePoint` deliberately carry no series,
        /// so a single point can never grow a chart.
        landed_hours: Vec<f64>,
    },
}

/// Time-to-land, inseparable from its caveat.
///
/// Both fields are private; see the module docs for why, and for the
/// `compile_fail` doctest that proves it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeToLand {
    /// Always serialized. No `skip_serializing_if`, no `Option`, no
    /// variant without it.
    caveat: &'static str,
    kind: TimeToLandKind,
}

impl TimeToLand {
    /// Folds the landed cycles. Order of `cycles` does not matter — they
    /// are sorted by landing time here, since "as the knowledge base grows"
    /// is a statement about landing order.
    pub fn from_cycles(mut cycles: Vec<FeatureCycle>) -> Self {
        cycles.sort_by_key(|c| c.landed);
        let kind = match cycles.len() {
            0 => TimeToLandKind::NoData,
            1 => TimeToLandKind::SinglePoint {
                hours: cycles[0].hours(),
            },
            n => {
                // Odd counts give the middle feature to the later half:
                // the question is "is it getting faster", so the fresher
                // side is the one that should not be starved.
                let split = n / 2;
                let mut earlier: Vec<f64> =
                    cycles[..split].iter().map(FeatureCycle::hours).collect();
                let mut later: Vec<f64> = cycles[split..].iter().map(FeatureCycle::hours).collect();
                let landed_hours: Vec<f64> = cycles.iter().map(FeatureCycle::hours).collect();
                TimeToLandKind::Trend {
                    earlier_median_hours: median(&mut earlier),
                    later_median_hours: median(&mut later),
                    landed: n as u32,
                    landed_hours,
                }
            }
        };
        Self {
            caveat: TREND_CAVEAT,
            kind,
        }
    }

    /// The only way to read the figures — and it hands you the caveat in
    /// the same expression. There is deliberately no accessor that returns
    /// the numbers alone. A clone, since `Trend` carries the per-cycle
    /// series and the enum is no longer `Copy`.
    pub fn read(&self) -> (TimeToLandKind, &'static str) {
        (self.kind.clone(), self.caveat)
    }

    /// One rendering that cannot lose the sentence.
    pub fn summary(&self) -> String {
        let (kind, caveat) = self.read();
        let head = match kind {
            TimeToLandKind::NoData => {
                "no landed feature yet has both a created and a merged breadcrumb".to_string()
            }
            TimeToLandKind::SinglePoint { hours } => format!(
                "one feature landed, in {hours:.1}h — a point, not a trend; there is nothing \
                 to compare it to yet"
            ),
            TimeToLandKind::Trend {
                earlier_median_hours,
                later_median_hours,
                landed,
                ..
            } => format!(
                "median time to land: {earlier_median_hours:.1}h for the earlier half, \
                 {later_median_hours:.1}h for the later half, over {landed} landed feature(s)"
            ),
        };
        format!("{head}. {caveat}")
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS` (UTC) to unix seconds.
///
/// Both shapes occur in a real dossier: the app writes minutes
/// (`dossiers::now_stamp`), while the backfilled `created` line copies
/// `issues.created_at`, which SQLite's `datetime('now')` renders with
/// seconds.
///
/// No date crate, matching `fartcode_core::dossiers` — the workspace has
/// none and one stamp parser does not justify adding one. This is the exact
/// inverse of the civil-from-days algorithm that wrote the stamp.
pub fn parse_stamp(stamp: &str) -> Option<i64> {
    let (date, time) = stamp.split_once(' ')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = match time_parts.next() {
        Some(s) => s.parse().ok()?,
        None => 0,
    };
    if time_parts.next().is_some()
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return None;
    }

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The date an agent-written dossier section carries, from its heading.
///
/// The convention is `## <Column> — <YYYY-MM-DD>` (seeded prompt and skill
/// both), so the date is the last whitespace-delimited token that parses as
/// one. Midnight UTC — a section is dated to the day, not the minute.
///
/// `None` for a heading that carries no date. Callers windowing by time
/// must **exclude** those rather than assume they are recent: including an
/// undated section is how a report labelled "the last 90 days" silently
/// starts spanning all history.
pub fn section_date(heading: &str) -> Option<i64> {
    heading.split_whitespace().rev().find_map(|token| {
        parse_stamp(&format!(
            "{} 00:00",
            token.trim_matches(|c: char| !c.is_ascii_digit() && c != '-')
        ))
    })
}

/// Howard Hinnant's `days_from_civil` — the inverse of the `civil_from_days`
/// in `fartcode_core::dossiers::format_stamp`.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600;

    /// A dossier Timeline exactly as `fartcode_core::dossiers` writes it,
    /// sentinel and all.
    const TIMELINE: &str = "\
<!-- fartcode:timeline -->

- 2026-08-01 09:00:00 · created · proposal · docs/prds/oauth.md
- 2026-08-01 09:05 · dossier created with the worktree · In Progress
- 2026-08-01 09:05 · In Progress · launched · claude
- 2026-08-02 11:00 · In Progress · settled
- 2026-08-02 12:00 · pr opened · https://example.test/pr/1
- 2026-08-03 09:00 · pr merged · https://example.test/pr/1
";

    #[test]
    fn parses_the_stamps_the_app_actually_writes() {
        // Minutes (dossiers::now_stamp) and seconds (SQLite created_at).
        assert_eq!(parse_stamp("1970-01-01 00:00"), Some(0));
        assert_eq!(parse_stamp("1970-01-01 00:00:01"), Some(1));
        // 2026-08-09 00:00:00 UTC is 1_786_233_600.
        assert_eq!(parse_stamp("2026-08-09 00:00"), Some(1_786_233_600));
        assert_eq!(parse_stamp("2026-08-09 12:34"), Some(1_786_278_840));
        assert_eq!(parse_stamp("2026-08-09 12:34:56"), Some(1_786_278_896));
        // Leap day round-trips.
        assert_eq!(
            parse_stamp("2024-03-01 00:00").unwrap() - parse_stamp("2024-02-29 00:00").unwrap(),
            86_400
        );
        // Junk is rejected, never coerced.
        for bad in [
            "",
            "not a stamp",
            "2026-13-01 00:00",
            "2026-08-09",
            "2026-08-09 25:00",
            "2026-08-09 00:61",
            "2026-08-09 00:00:60",
            "2026-08-09 00:00:00:00",
        ] {
            assert_eq!(parse_stamp(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn timeline_parsing_skips_everything_that_is_not_a_breadcrumb() {
        let events = parse_timeline(TIMELINE);
        assert_eq!(events.len(), 6);
        assert_eq!(events[0].fact, "created · proposal · docs/prds/oauth.md");
        assert_eq!(events[5].fact, "pr merged · https://example.test/pr/1");
        // The sentinel comment and the blank line contributed nothing.
        assert!(!events.iter().any(|e| e.fact.contains("fartcode:timeline")));
    }

    #[test]
    fn a_cycle_runs_from_created_to_pr_merged() {
        let cycle = cycle_of(&parse_timeline(TIMELINE)).unwrap();
        assert_eq!(cycle.created, parse_stamp("2026-08-01 09:00:00").unwrap());
        assert_eq!(cycle.landed, parse_stamp("2026-08-03 09:00").unwrap());
        assert_eq!(cycle.hours(), 48.0);
    }

    #[test]
    fn an_unmerged_feature_has_no_cycle() {
        let unmerged = "\
- 2026-08-01 09:00 · created · manual
- 2026-08-02 11:00 · In Progress · settled
- 2026-08-02 12:00 · pr opened · https://example.test/pr/9
";
        assert_eq!(cycle_of(&parse_timeline(unmerged)), None);
        // ...and neither does one with no created line.
        let headless = "- 2026-08-03 09:00 · pr merged · https://example.test/pr/9";
        assert_eq!(cycle_of(&parse_timeline(headless)), None);
    }

    #[test]
    fn a_backwards_cycle_is_refused_rather_than_counted_negative() {
        let skewed = "\
- 2026-08-05 09:00 · created · manual
- 2026-08-01 09:00 · pr merged · https://example.test/pr/9
";
        assert_eq!(cycle_of(&parse_timeline(skewed)), None);
    }

    fn cycle(start_day: i64, hours: i64) -> FeatureCycle {
        let created = start_day * 86_400;
        FeatureCycle {
            created,
            landed: created + hours * HOUR,
        }
    }

    #[test]
    fn section_dates_come_off_the_heading_or_are_absent() {
        let day = parse_stamp("2026-08-09 00:00").unwrap();
        assert_eq!(section_date("Plan — 2026-08-09"), Some(day));
        assert_eq!(section_date("In Review — 2026-08-09"), Some(day));
        // Punctuation around the token is tolerated.
        assert_eq!(section_date("Plan (2026-08-09)"), Some(day));
        // No date is None — never "assume it is recent".
        assert_eq!(section_date("Plan"), None);
        assert_eq!(section_date("Context"), None);
        assert_eq!(section_date("Plan — yesterday"), None);
    }

    #[test]
    fn no_landed_features_is_no_data() {
        let (kind, caveat) = TimeToLand::from_cycles(Vec::new()).read();
        assert_eq!(kind, TimeToLandKind::NoData);
        assert_eq!(caveat, TREND_CAVEAT);
    }

    /// The degenerate case the ticket asks about by name.
    #[test]
    fn one_feature_is_a_point_and_says_so() {
        let value = TimeToLand::from_cycles(vec![cycle(1, 30)]);
        let (kind, _) = value.read();
        assert_eq!(kind, TimeToLandKind::SinglePoint { hours: 30.0 });
        let summary = value.summary();
        assert!(summary.contains("a point, not a trend"), "{summary}");
        assert!(
            summary.contains("nothing to compare it to yet"),
            "{summary}"
        );
    }

    #[test]
    fn a_trend_reports_two_medians_and_its_sample_size() {
        // Landing order is what matters, so hand them in shuffled.
        let value = TimeToLand::from_cycles(vec![
            cycle(40, 20),
            cycle(10, 100),
            cycle(30, 30),
            cycle(20, 80),
        ]);
        let (kind, _) = value.read();
        assert_eq!(
            kind,
            TimeToLandKind::Trend {
                earlier_median_hours: 90.0,
                later_median_hours: 25.0,
                landed: 4,
                // Landing order — cycle(10,…) landed first, cycle(40,…) last.
                landed_hours: vec![100.0, 80.0, 30.0, 20.0],
            }
        );
        // No percentage, no slope — the caveat forbids the attribution one
        // number would invite.
        let summary = value.summary();
        assert!(!summary.contains('%'), "{summary}");
    }

    #[test]
    fn two_features_split_one_each() {
        let value = TimeToLand::from_cycles(vec![cycle(1, 10), cycle(2, 40)]);
        assert_eq!(
            value.read().0,
            TimeToLandKind::Trend {
                earlier_median_hours: 10.0,
                later_median_hours: 40.0,
                landed: 2,
                landed_hours: vec![10.0, 40.0],
            }
        );
    }

    /// #76's sparkline series: Trend serializes the per-cycle hours in
    /// landing order; NoData and SinglePoint carry no series at all; and
    /// every serialization still contains "caveat".
    #[test]
    fn only_a_trend_serializes_a_landing_ordered_series() {
        let trend = TimeToLand::from_cycles(vec![cycle(30, 5), cycle(10, 90), cycle(20, 40)]);
        let json = serde_json::to_string(&trend).unwrap();
        assert!(json.contains("\"landedHours\":[90.0,40.0,5.0]"), "{json}");

        for value in [
            TimeToLand::from_cycles(Vec::new()),
            TimeToLand::from_cycles(vec![cycle(1, 12)]),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            assert!(!json.contains("landedHours"), "{json}");
            assert!(json.contains("\"caveat\""), "{json}");
        }
        let json = serde_json::to_string(&trend).unwrap();
        assert!(json.contains("\"caveat\""), "{json}");
    }

    /// The caveat is inseparable from the value: it is in `read`, in
    /// `summary`, and in every serialization of every variant. (That the
    /// fields cannot be reached directly is proved by the `compile_fail`
    /// doctest in the module docs.)
    #[test]
    fn the_number_cannot_be_obtained_without_the_caveat() {
        for value in [
            TimeToLand::from_cycles(Vec::new()),
            TimeToLand::from_cycles(vec![cycle(1, 12)]),
            TimeToLand::from_cycles(vec![cycle(1, 12), cycle(2, 24)]),
        ] {
            assert_eq!(value.read().1, TREND_CAVEAT);
            assert!(value.summary().contains(TREND_CAVEAT));
            let json = serde_json::to_string(&value).unwrap();
            assert!(json.contains("\"caveat\""), "{json}");
            assert!(json.contains("Trend, not attribution"), "{json}");
        }
    }
}
