// First-dispatch feature-dossier consent card (E19-05, #74; handoff v3
// §8e, ADR-0038 item 3).
//
// **This card is what turns the dossier feature on.** E19-01/02 shipped
// fail-closed: `feature_dossiers` is `Option<bool>` and `None` — never
// asked — resolves to "don't write" in `fartcode-app/src/dossiers.rs`
// `consented()`. Until an answer lands, the whole feature is inert for
// every project. So the ONE invariant is that both answers persist an
// explicit boolean: `↵` writes `true`, `esc` writes `false`, and neither
// leaves the setting `null`. A decline that stayed `null` would ask again
// on the next dispatch, forever.
//
// It renders app-level (App.tsx, beside Onboarding) rather than inside the
// board, because two of the three surfaces that start an agent step live
// in the task view, where BoardView is unmounted. The gate and its state
// live in `store/dossierConsent.ts`.
//
// **The backdrop is inert, on purpose.** The chrome is the board confirm's
// chrome, where clicking outside means "keep it where it is" — it changes
// nothing and spends nothing. Here the only dismissal is a decline, and a
// decline is permanent: it writes `false` and still launches an agent.
// Wiring the largest click target on screen to that would let a reflexive
// click-away opt a repo out of dossiers for good AND dispatch. So the
// backdrop has no handler at all, matching Onboarding.tsx — the repo's
// precedent for a must-answer overlay. Esc still declines, because `esc
// run without memory` is §8e's own grammar and the footer says so.

import { useEffect } from "react";
import { dossierPathFor, useDossierConsent } from "../store/dossierConsent";

// A failed consent write is NOT surfaced here: the card is dismissed the
// moment the answer is given (the dispatch must not wait on a dialog), so
// by the time the write can fail there is nothing on screen to attach the
// message to. It lives in the store and BoardView renders it on its
// existing error line, beside the step-engine errors it already shows.

/** The convention files, in the order §8e lists them. `AGENTS.md` is the
 * odd one out — the app appends a single pointer line rather than owning
 * the file — so it says so instead of implying ownership. */
function conventionFiles(dossierPath: string): string[] {
  return [dossierPath, ".claude/skills/feature-log/", "AGENTS.md · one pointer line"];
}

export default function DossierConsentCard() {
  const ask = useDossierConsent((s) => s.ask);
  const answer = useDossierConsent((s) => s.answer);

  useEffect(() => {
    if (!ask) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        answer(true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        answer(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ask, answer]);

  if (!ask) return null;

  return (
    <div className="board-confirm-backdrop">
      <div
        className="board-confirm board-consent"
        role="alertdialog"
        aria-label="Keep a feature dossier in this repo"
      >
        <div className="board-confirm-body board-consent-body">
          This feature will keep a dossier — write the convention files to your repo?
        </div>
        {/* One stack: the file list and the provenance line share the
            §8e rhythm, so the provenance reads as the list's last line
            rather than as a second block. */}
        <div className="board-consent-stack">
          <ul className="board-consent-files">
            {conventionFiles(dossierPathFor(ask.issue)).map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
          <div className="board-consent-note">
            provenance-tagged · commits ride the feature branch
          </div>
        </div>
        <div className="board-confirm-foot">
          <button type="button" onClick={() => answer(false)}>
            esc run without memory
          </button>
          <button type="button" onClick={() => answer(true)}>
            <span className="board-confirm-key">↵</span> write to repo
          </button>
        </div>
      </div>
    </div>
  );
}
