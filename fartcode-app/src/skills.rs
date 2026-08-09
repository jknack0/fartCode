//! Seeding the feature-log convention, and injecting its other half into
//! step prompts (E19-02, #71; ADR-0038 items 2–3).
//!
//! The file content and the scaffold surgery live in
//! `fartcode_core::skills`. This module is the half that needs the wired
//! App: the consent gate and the two call sites.
//!
//! **Where the scaffold is written: the WORKTREE, not the main checkout.**
//! Same place the dossier is born, and for the same three reasons.
//! (1) *Reviewability* — the dispatch prompt tells agents to commit as they
//! go, so a worktree write lands in the user's pull request where they can
//! read it and drop it; the main checkout has no such review surface, and a
//! write there is a silent mutation of a working tree that may be mid-edit
//! on an unrelated branch. (2) *ADR-0038 item 5, "merge is publication"* —
//! the convention reaches the default branch exactly when the first feature
//! that used it lands, so the skill and the dossiers it describes arrive
//! together, never a skill describing files that do not exist. (3) *No
//! branch surprise* — writing `AGENTS.md` into the checkout would sweep
//! into whatever commit the user makes next.
//!
//! The accepted cost, stated rather than hidden: an agent CLI run in the
//! main checkout does not see the convention until the first feature
//! merges. That is the same delay the dossiers themselves have, and the
//! alternative is writing into a tree the user is standing in. A second,
//! smaller one: a project that gitignores `.claude/` keeps the skill
//! locally but never publishes it, so only the `AGENTS.md` pointer travels.
//! Un-ignoring someone's `.claude/` to fix that would be a bigger liberty
//! than the problem.
//!
//! **Consent gates everything here.** Both entry points call
//! [`crate::dossiers::consented`] — the one gate, reused, never
//! reimplemented. Seeding into a project that declined would write the
//! files we refused to write; injecting the append instruction there would
//! be worse, because the agent would then create the dossier itself and it
//! would arrive in the user's PR with nothing to trace it back to.

use std::path::Path;

use fartcode_core::issues::Issue;
use fartcode_core::projects::ProjectStore;
use fartcode_core::skills;

use crate::app::App;

/// Scaffolds `.claude/skills/feature-log/` + the `AGENTS.md` pointer into a
/// task's worktree, if the project consented and the scaffold is not
/// already recorded as current.
///
/// **The only seeding entry point.** Called from
/// `dispatch::provision_issue_task` (the shared provisioning tail) and from
/// `step_engine::launch_step`'s non-reattach branch — the second matters
/// because a card whose worktree already exists never re-provisions, so
/// without it a [`skills::FEATURE_LOG_VERSION`] bump would never reach a
/// feature already in flight.
///
/// Three gates, in increasing cost, all of them cheap:
///
/// 1. **Consent** — the same fail-closed gate the dossier uses.
/// 2. **`feature_log_seeded_version`** — the app's memory of what it
///    already wrote here. This is what makes the removal instructions
///    printed inside the scaffold TRUE: without it, a user who deletes the
///    files gets them back on the next card, forever, and the sentence
///    "delete this directory to remove the convention" is a lie that costs
///    them a diff in every future pull request. A version bump makes the
///    comparison fail again, so a real format change still heals.
/// 3. The filesystem inspection in [`skills::seed`] itself.
///
/// Best-effort by contract, exactly like the dossier: every failure is a
/// `tracing::warn!` and nothing else. A read-only repo must not fail a
/// dispatch.
pub fn seed_for_task(app: &App, project_id: &str, task_id: &str) {
    if !crate::dossiers::consented(app, project_id) {
        tracing::debug!(
            project_id,
            "feature dossiers off for this project — not seeding the skill"
        );
        return;
    }
    if seeded_version(app, project_id).is_some_and(|v| v >= skills::FEATURE_LOG_VERSION) {
        // Already done at this version. If the files are gone, the user
        // removed them and we respect that.
        return;
    }
    let Some(worktree) = crate::dossiers::task_worktree(app, task_id) else {
        return;
    };
    match skills::seed(&worktree) {
        Ok(report) if report.wrote() => {
            tracing::info!(
                worktree = %worktree.display(),
                skill = ?report.skill,
                pointer = ?report.pointer,
                version = skills::FEATURE_LOG_VERSION,
                "feature-log convention seeded"
            );
            // Recorded ONLY after a write actually landed. A refusal or a
            // decline (the paths are the user's) must not be remembered as
            // "done" — the next launch should look again.
            record_seeded_version(app, project_id, skills::FEATURE_LOG_VERSION);
        }
        // Up to date, or the paths belong to someone else. Both are
        // successful outcomes and neither is worth a line at info.
        Ok(report) => tracing::debug!(
            worktree = %worktree.display(),
            skill = ?report.skill,
            pointer = ?report.pointer,
            "feature-log convention not written"
        ),
        Err(e) => tracing::warn!(
            worktree = %worktree.display(),
            error = %e,
            "seeding the feature-log convention failed"
        ),
    }
}

/// The scaffold version this project was last successfully seeded at.
fn seeded_version(app: &App, project_id: &str) -> Option<u32> {
    let project = app.projects.get(project_id).ok().flatten()?;
    app.settings
        .get_project_settings(project_id, Path::new(&project.path))
        .ok()?
        .feature_log_seeded_version
}

/// Records a successful seed. Read-modify-write, because
/// `update_project_settings` is full-replace — sending a partial object
/// would clear every field we did not name.
fn record_seeded_version(app: &App, project_id: &str, version: u32) {
    let Ok(Some(project)) = app.projects.get(project_id) else {
        return;
    };
    let repo = Path::new(&project.path);
    let mut settings = match app.settings.get_project_settings(project_id, repo) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(project_id, error = %e, "cannot read settings to record the seed");
            return;
        }
    };
    settings.feature_log_seeded_version = Some(version);
    if let Err(e) = app
        .settings
        .update_project_settings(project_id, repo, &settings)
    {
        // Losing this only costs one redundant no-op seed next launch.
        tracing::warn!(project_id, error = %e, "recording the seeded scaffold version failed");
    }
}

/// Appends the ADR-0038 item 2 instruction — "before settling, add
/// `## <Column> — <date>` with decisions, tradeoffs and rejected
/// alternatives" — to a composed step prompt.
///
/// Two conditions, both required, both about not lying to the agent:
///
/// 1. **Consent.** Checked here rather than at the call sites so there is
///    one place to forget it. Consent is re-read per launch, so revoking it
///    stops the instruction on the very next step — an instruction issued
///    under an old "yes" would outlive the answer.
/// 2. **The card actually has a dossier.** `dossier_path` is NULL when
///    creation was refused or failed — including the case where the slug
///    was occupied by a human's document and the app declined to touch it.
///    Naming a path in the prompt that the app itself would not write is
///    how an agent ends up clobbering that document. No file, no
///    instruction.
///
/// The instruction goes at the very END of the prompt (ADR-0038 item 2:
/// "each seeded step prompt *ends* with the append instruction"), after any
/// column framing and after the reference packet, under the same `---`
/// divider `compose_step_prompt` uses.
pub fn with_append_instruction(
    app: &App,
    issue: &Issue,
    column_name: &str,
    prompt: String,
) -> String {
    let Some(rel) = issue
        .dossier_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return prompt;
    };
    if !crate::dossiers::consented(app, &issue.project_id) {
        return prompt;
    }
    format!(
        "{prompt}\n\n---\n\n{}",
        skills::append_instruction(column_name, rel)
    )
}
