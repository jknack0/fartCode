// First-dispatch feature-dossier consent (E19-05, #74; handoff v3 §8e,
// ADR-0038 item 3).
//
// **This card is what turns the dossier feature on.** E19-01/02 shipped
// fail-closed: `feature_dossiers` is `Option<bool>` and `None` — never
// asked — resolves to "don't write" in `fartcode-app/src/dossiers.rs`
// `consented()`. Until an answer lands, the whole feature is inert for
// every project. So the ONE invariant here is that both answers persist an
// explicit boolean: `↵` writes `true`, `esc` writes `false`, and neither
// leaves the setting `null`. A decline that stayed `null` would ask again
// on the next dispatch, forever.
//
// Two more rules the copy encodes:
//   - **Declining still dispatches.** The card gates the repo WRITE, never
//     the agent. `esc run without memory` is a literal promise; BoardView
//     carries the deferred entry out on both answers.
//   - **Never asked again either way.** Reversal lives in project settings
//     (`feature dossiers · on|off`), not in a second prompt.
//
// The chrome is deliberately the board confirm's chrome — same backdrop,
// same overlay card, same key-first footer grammar (`.board-confirm*`).
// A consent overlay that looked like its own dialog system would be a
// second design.

import { getProjectSettings, updateProjectSettings, type IssueDto } from "../../lib/tauri";

/** Consent as the backend models it: `true`/`false` are answers, `null` is
 * "never asked" — the state this card exists to clear. */
export type DossierConsent = boolean | null;

/** Mirror of `fartcode_core::tasks::naming::sanitize_name`: lowercase,
 * every non-alphanumeric run to a single `-`, trimmed, capped at 64. The
 * card names a real file, so the two slugifiers must agree. */
const MAX_SLUG_LENGTH = 64;

function sanitizeSlug(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, MAX_SLUG_LENGTH);
}

/** The dossier slug for a card — `dossier_slug()` in
 * `fartcode-core/src/dossiers.rs`: the title slugified the way task names
 * are, falling back to the card id when the title sanitizes to nothing
 * (all punctuation, all emoji). */
export function dossierSlug(issue: Pick<IssueDto, "id" | "title">): string {
  return sanitizeSlug(issue.title) || sanitizeSlug(issue.id);
}

/** The repo-relative path the card promises, e.g.
 * `docs/features/oauth-login.md`. An already-created dossier names itself;
 * on the FIRST dispatch — which is the only time this card renders — there
 * is none yet, so the slug is derived. The backend may still append a
 * `-<short id>` disambiguator if that exact filename is taken, which is
 * why the app never treats this string as the authority. */
export function dossierPathFor(
  issue: Pick<IssueDto, "id" | "title" | "dossierPath">,
): string {
  return issue.dossierPath || `docs/features/${dossierSlug(issue)}.md`;
}

/** Reads this project's consent. Errors read as `null` (never asked)
 * rather than as a grant — the same fail-closed posture as the backend. */
export async function readDossierConsent(projectId: string): Promise<DossierConsent> {
  try {
    const settings = await getProjectSettings(projectId);
    return settings.featureDossiers ?? null;
  } catch {
    return null;
  }
}

/** Persists the answer.
 *
 * **Read-modify-write, deliberately.** `update_project_settings` is
 * FULL-REPLACE (fartcode-core/src/settings/service.rs): whatever object
 * this sends becomes the whole stored row. Writing a hand-built object
 * would clear `feature_log_seeded_version` — the app's memory of the
 * scaffold it already seeded — and resurrect files the user deleted. So
 * the answer is merged onto a FRESH read, never onto a cached copy. */
export async function writeDossierConsent(
  projectId: string,
  consented: boolean,
): Promise<void> {
  const fresh = await getProjectSettings(projectId);
  await updateProjectSettings(projectId, { ...fresh, featureDossiers: consented });
}

/** The convention files, in the order §8e lists them. `AGENTS.md` is the
 * odd one out — the app appends a single pointer line rather than owning
 * the file — so it says so instead of implying ownership. */
function conventionFiles(dossierPath: string): string[] {
  return [dossierPath, ".claude/skills/feature-log/", "AGENTS.md · one pointer line"];
}

export default function DossierConsentCard({
  issue,
  onWrite,
  onDecline,
}: {
  issue: Pick<IssueDto, "id" | "title" | "dossierPath">;
  /** `↵ write to repo` — persist `true`, then carry on. */
  onWrite: () => void;
  /** `esc run without memory` — persist `false`, then carry on ANYWAY. */
  onDecline: () => void;
}) {
  return (
    <div className="board-confirm-backdrop" onClick={onDecline}>
      <div
        className="board-confirm board-consent"
        role="alertdialog"
        aria-label="Keep a feature dossier in this repo"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="board-confirm-body board-consent-body">
          This feature will keep a dossier — write the convention files to your repo?
        </div>
        <ul className="board-consent-files">
          {conventionFiles(dossierPathFor(issue)).map((f) => (
            <li key={f}>{f}</li>
          ))}
        </ul>
        <div className="board-consent-note">
          provenance-tagged · commits ride the feature branch
        </div>
        <div className="board-confirm-foot">
          <button type="button" onClick={onDecline}>
            esc run without memory
          </button>
          <button type="button" onClick={onWrite}>
            <span className="board-confirm-key">↵</span> write to repo
          </button>
        </div>
      </div>
    </div>
  );
}
