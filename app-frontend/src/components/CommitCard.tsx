// Commit card (E4-06, restyled to design_handoff_v2 §5d): inset card at the
// bottom of the Changes sidebar — bare message input (idle fc-caret ghost,
// native caret in accent once focused) over three key-first rows:
// Commit ↵ / Commit & push ⌘↵ / Commit, push & open PR ⌘⇧↵. All three key
// combos work from the input. Disabled states derive from the backend
// CommitState (branch, push remote, PR-open guard); git errors surface
// inline and the message stays for retry. The card commits exactly the
// staged set — stage-all lives on the Changed section label ("a stage all").
import { useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import { gitCreatePr } from "../lib/tauri";
import { useChanges } from "../store/changes";
import { useCommitState } from "../store/commit-state";

type Phase = "idle" | "committing" | "pushing" | "creating-pr";

export default function CommitCard({ workspaceId }: { workspaceId: string }) {
  const stateEntry = useCommitState((s) => s.byWorkspace[workspaceId]);
  const changesEntry = useChanges((s) => s.byWorkspace[workspaceId]);
  const [message, setMessage] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);

  const st = stateEntry?.state ?? null;
  const busy = phase !== "idle";
  const stagedCount = changesEntry?.snapshot?.staged.length ?? 0;
  // Disabled matrix (ticket): empty message / nothing staged / detached HEAD;
  // push additionally needs a push remote.
  const commitDisabled = busy || message.trim().length === 0 || !st?.branch || stagedCount === 0;
  const pushDisabled = commitDisabled || !st?.hasRemote;

  const fail = (e: unknown) => {
    setError(String(e));
    setPhase("idle");
  };

  const doCommit = async (then: "push" | "pr" | null) => {
    setError(null);
    setPhase("committing");
    try {
      await useCommitState.getState().commit(workspaceId, message);
      setMessage("");
      void useChanges.getState().refetch(workspaceId);
    } catch (e) {
      fail(e);
      return;
    }
    if (then === "push") {
      setPhase("pushing");
      try {
        await useCommitState.getState().push(workspaceId);
        setPhase("idle");
      } catch (e) {
        fail(e);
      }
      return;
    }
    if (then === "pr") {
      setPhase("creating-pr");
      try {
        // Push-when-unpublished + PR-open guard live in the backend
        // (fartcode_git::commit::create_pr); the browser finishes the PR.
        const outcome = await gitCreatePr(workspaceId);
        await open(outcome.url);
        setPhase("idle");
      } catch (e) {
        fail(e);
      }
      return;
    }
    setPhase("idle");
  };

  if (!st) return null;

  const commitTitle = !st.branch
    ? "HEAD is detached — nothing to commit"
    : stagedCount === 0
      ? "Nothing staged"
      : `Commit the ${stagedCount} staged file${stagedCount === 1 ? "" : "s"}`;
  const pushTitle = !st.hasRemote
    ? "No push remote configured"
    : st.published
      ? `Push to ${st.remote}`
      : `Push and set upstream on ${st.remote}`;

  return (
    <div className="fc-commit-card">
      <div className="fc-commit-input">
        <input
          className="fc-commit-message"
          aria-label="Commit message"
          value={message}
          disabled={busy}
          onChange={(e) => setMessage(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            e.stopPropagation();
            if (e.metaKey && e.shiftKey) {
              if (!commitDisabled && st.canCreatePr) void doCommit("pr");
            } else if (e.metaKey) {
              if (!pushDisabled) void doCommit("push");
            } else if (!commitDisabled) {
              void doCommit(null);
            }
          }}
        />
        {message === "" && (
          <span className="fc-commit-ghost" aria-hidden>
            <i className="fc-ghost-caret" />
            Commit message
          </span>
        )}
      </div>
      <div className="fc-commit-keys">
        <div
          className={`fc-krow fc-krow-primary${commitDisabled ? " disabled" : ""}`}
          role="button"
          aria-disabled={commitDisabled}
          title={commitTitle}
          onClick={() => {
            if (!commitDisabled) void doCommit(null);
          }}
        >
          <span className="fc-krow-label">
            {phase === "committing" ? "Committing…" : "Commit"}
          </span>
          <span className="fc-krow-key">↵</span>
        </div>
        <div
          className={`fc-krow${pushDisabled ? " disabled" : ""}`}
          role="button"
          aria-disabled={pushDisabled}
          title={pushTitle}
          onClick={() => {
            if (!pushDisabled) void doCommit("push");
          }}
        >
          <span className="fc-krow-label">
            {phase === "pushing" ? "Pushing…" : "Commit & push"}
          </span>
          <span className="fc-krow-key">⌘↵</span>
        </div>
        {/* PR-open guard (reference): when a PR is already open the action
            degrades to push-only — the row is replaced by the note. */}
        {st.canCreatePr ? (
          <div
            className={`fc-krow${commitDisabled ? " disabled" : ""}`}
            role="button"
            aria-disabled={commitDisabled}
            onClick={() => {
              if (!commitDisabled) void doCommit("pr");
            }}
          >
            <span className="fc-krow-label">
              {phase === "creating-pr" ? "Opening PR…" : "Commit, push & open PR"}
            </span>
            <span className="fc-krow-key">⌘⇧↵</span>
          </div>
        ) : (
          st.prOpen && <div className="fc-commit-note">PR already open — push instead</div>
        )}
      </div>
      {error && (
        <p className="fc-commit-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
