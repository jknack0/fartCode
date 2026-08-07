// Commit card (E4-06): bottom of the Changes sidebar — commit message +
// Commit / Commit & Push / Commit & Create PR. Disabled states derive from
// the backend CommitState (branch, push remote, PR-open guard); git errors
// surface inline and the message stays for retry. Deviations from the
// reference commit-card: no autoStage (the card commits exactly the staged
// set — "Stage all" lives in the panel header), no description field, and
// explicit buttons instead of a split button with a remembered action.
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

  const commitLabel =
    phase === "committing"
      ? "Committing…"
      : phase === "pushing"
        ? "Pushing…"
        : phase === "creating-pr"
          ? "Opening PR…"
          : "Commit";

  if (!st) return null;

  return (
    <div className="commit-card">
      <input
        className="commit-message"
        placeholder="Commit message"
        value={message}
        disabled={busy}
        onChange={(e) => setMessage(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !commitDisabled) void doCommit(null);
        }}
      />
      <div className="commit-card-actions">
        <button
          className="primary commit-action"
          disabled={commitDisabled}
          title={
            !st.branch
              ? "HEAD is detached — nothing to commit"
              : stagedCount === 0
                ? "Nothing staged"
                : `Commit the ${stagedCount} staged file${stagedCount === 1 ? "" : "s"}`
          }
          onClick={() => void doCommit(null)}
        >
          {commitLabel}
        </button>
        <button
          className="commit-action"
          disabled={pushDisabled}
          title={
            !st.hasRemote
              ? "No push remote configured"
              : st.published
                ? `Push to ${st.remote}`
                : `Push and set upstream on ${st.remote}`
          }
          onClick={() => void doCommit("push")}
        >
          Commit &amp; Push
        </button>
        {/* PR-open guard (reference): when a PR is already open the action
            degrades to push-only — the button is replaced by the note. */}
        {st.canCreatePr ? (
          <button
            className="commit-action"
            disabled={commitDisabled}
            onClick={() => void doCommit("pr")}
          >
            Commit &amp; Create PR
          </button>
        ) : (
          st.prOpen && <span className="commit-note">PR already open — push instead</span>
        )}
      </div>
      {error && <p className="commit-error" role="alert">{error}</p>}
    </div>
  );
}
