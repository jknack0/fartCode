// Feature-dossier consent, as an APP-LIFETIME gate (E19-05, #74; handoff
// v3 §8e, ADR-0038 item 3).
//
// This lived inside BoardView first, and that was the wrong lifetime twice
// over.
//
// *Wrong in space*: three surfaces start an agent step, and only one of
// them is the board. `taskPipeline.enterColumn` makes the same
// `issue_enter_column` call a board drag makes ("because it is the same
// call", says its own comment), and `CardDetail`'s Dispatch runs
// `issue_dispatch`, which provisions the worktree — literally the moment
// §8e names. A board-owned overlay cannot render for either: the task view
// has BoardView unmounted. A user who drives the pipeline from the task
// view would never be asked, and the feature would stay inert forever with
// nothing on screen to explain why.
//
// *Wrong in time*: the answer was written with the `projectId` captured in
// the render closure that RAISED the card, but BoardView is not remounted
// on a project switch (App.tsx renders ProjectView with no key). Raise the
// card in project A, click project B, press ↵ — and the consent landed on
// B, a project nobody asked about, possibly flipping an explicit decline
// back to true. Consent belongs to the project that was ASKED, so the ask
// carries its own project id and the write uses that, never ambient state.
//
// The gate is therefore one function — `ensureDossierConsent` — that every
// dispatch path awaits, and which resolves to "may I proceed". It is NOT
// "did they consent": declining still dispatches (§8e), so the boolean is
// about the gesture, not the answer. Only a cancelled ask (the project was
// switched out from under it) returns false.

import { create } from "zustand";
import { getProjectSettings, updateProjectSettings, type IssueDto } from "../lib/tauri";

/** What a settings read can tell us. `null` is the state this whole
 * feature exists to clear; `"unreadable"` is deliberately NOT the same
 * thing (see `resolveConsent`). */
export type DossierConsent = boolean | null | "unreadable";

/** A project's settled state: an answer, or a read we could not perform. */
type Settled = boolean | "unreadable";

/** The card that is currently on screen, and the project it is asking
 * about — the project the answer will be written to. */
export interface DossierAsk {
  projectId: string;
  issue: Pick<IssueDto, "id" | "title" | "dossierPath">;
}

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

/** Reads this project's consent.
 *
 * A failed read is `"unreadable"`, NOT `null`. They look alike and behave
 * oppositely: `null` means "never asked", which must raise the card, while
 * a broken IPC or settings row would then raise it on every single entry,
 * forever — a permission prompt the user cannot dismiss by answering it,
 * because the answer cannot be stored either. So an unreadable row is
 * treated as "don't ask, just dispatch", which is safe precisely because
 * the backend fails closed at launch: `consented()` reads the same
 * unreadable row and writes nothing. */
export async function readDossierConsent(projectId: string): Promise<DossierConsent> {
  try {
    const settings = await getProjectSettings(projectId);
    return settings.featureDossiers ?? null;
  } catch {
    return "unreadable";
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

interface DossierConsentState {
  /** Per-project settled state. A key's presence means "do not ask". */
  byProject: Record<string, Settled>;
  /** The card on screen, or null. */
  ask: DossierAsk | null;
  /** A failed consent write — surfaced, never swallowed. */
  error: string | null;
  /** `↵ write to repo` / `esc run without memory`. Both persist an
   * explicit boolean; both then let the caller dispatch. */
  answer: (write: boolean) => void;
  /** Project switch: an ask about a project we are no longer looking at is
   * withdrawn, unanswered, and its caller does NOT dispatch. */
  cancelForeignAsk: (mountedProjectId: string | null) => void;
  /** Test seam — the module holds process-lifetime state by design. */
  reset: () => void;
}

/** Waiters, by project, resolved when their ask settles. */
const waiters = new Map<string, (proceed: boolean) => void>();
/** In-flight `ensureDossierConsent` calls, by project. This is what makes
 * a double-raise impossible: two rapid entries (the synchronous window
 * before either has awaited anything) join ONE promise and see ONE card.
 * A guard local to any single caller could not do this — the two entries
 * can come from different surfaces. */
const inFlight = new Map<string, Promise<boolean>>();
/** Asks are presented one at a time; a second project's ask waits its
 * turn rather than overwriting the card on screen. */
let askQueue: Promise<unknown> = Promise.resolve();

function settle(projectId: string, proceed: boolean): void {
  const waiter = waiters.get(projectId);
  waiters.delete(projectId);
  waiter?.(proceed);
}

export const useDossierConsent = create<DossierConsentState>((set, get) => ({
  byProject: {},
  ask: null,
  error: null,

  answer: (write) => {
    const ask = get().ask;
    if (!ask) return;
    set({ ask: null, error: null });
    // The cache is poked only on SUCCESS, and the waiters are released
    // only once the write has settled. Flipping the cache optimistically
    // let the park-reconcile path raise its queue confirm — interactive,
    // ↵-able — before the consent write had even been issued, so a fast ↵↵
    // could launch a step whose consent had not landed. One barrier, both
    // paths behind it.
    void writeDossierConsent(ask.projectId, write)
      .then(() => {
        set((s) => ({ byProject: { ...s.byProject, [ask.projectId]: write } }));
      })
      .catch((e) => {
        // Not cached: the answer did not land, so the next entry asks
        // again rather than pretending a project consented to writes the
        // backend will (correctly) refuse to make.
        set({ error: String(e) });
      })
      .finally(() => settle(ask.projectId, true));
  },

  cancelForeignAsk: (mountedProjectId) => {
    const ask = get().ask;
    if (!ask || ask.projectId === mountedProjectId) return;
    set({ ask: null });
    settle(ask.projectId, false);
  },

  reset: () => {
    waiters.clear();
    inFlight.clear();
    askQueue = Promise.resolve();
    set({ byProject: {}, ask: null, error: null });
  },
}));

/** Has this project settled, without asking anything? */
export function dossierConsentSettled(projectId: string): boolean {
  return useDossierConsent.getState().byProject[projectId] !== undefined;
}

function presentAsk(ask: DossierAsk): Promise<boolean> {
  const run = askQueue.then(
    () =>
      new Promise<boolean>((resolve) => {
        waiters.set(ask.projectId, resolve);
        useDossierConsent.setState({ ask });
      }),
  );
  askQueue = run.catch(() => undefined);
  return run;
}

/** **THE gate.** Await this before any call that can start an agent step;
 * proceed when it resolves true.
 *
 * True means "carry on", which includes a decline — §8e is explicit that
 * declining still dispatches, because the card gates the repo write and
 * never the agent. False means the ask was withdrawn (the project was
 * switched away), and the caller must do nothing at all.
 *
 * It resolves without a card whenever the project has already settled, so
 * the common path costs one cached lookup. */
export function ensureDossierConsent(
  projectId: string,
  issue: Pick<IssueDto, "id" | "title" | "dossierPath">,
): Promise<boolean> {
  if (dossierConsentSettled(projectId)) return Promise.resolve(true);
  const existing = inFlight.get(projectId);
  if (existing) return existing;

  const run = (async () => {
    // The cache is not the authority: another surface (or the settings
    // pane) may have answered since this project was last touched.
    const read = await readDossierConsent(projectId);
    if (read !== null) {
      useDossierConsent.setState((s) => ({
        byProject: { ...s.byProject, [projectId]: read },
      }));
      return true;
    }
    return presentAsk({ projectId, issue });
  })();

  const tracked = run.finally(() => {
    if (inFlight.get(projectId) === tracked) inFlight.delete(projectId);
  });
  inFlight.set(projectId, tracked);
  return tracked;
}
