// Git footer (E4-08): everyday verbs under the commit card — Fetch / Pull /
// Push / Publish, plus the add-remote mini-form when the repo has no
// remotes. State comes from the shared CommitState DTO (git_commit_state);
// every action refetches it so affordances flip immediately (ticket:
// "footer flips to push/pull afterward"). Errors surface inline until the
// next successful action — the repo has no toast system.
import { useState } from "react";
import { useChanges } from "../store/changes";
import { useCommitState } from "../store/commit-state";

type Action = "fetch" | "pull" | "push" | "publish" | "add-remote";

export default function GitFooter({ workspaceId }: { workspaceId: string }) {
  const stateEntry = useCommitState((s) => s.byWorkspace[workspaceId]);

  const [busy, setBusy] = useState<Action | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [remoteName, setRemoteName] = useState("origin");
  const [remoteUrl, setRemoteUrl] = useState("");
  const st = stateEntry?.state ?? null;
  if (!st) return null;


  // Disabled matrix (ticket): driven by repo state, not by the UI.
  const fetchDisabled = busy !== null || !st.hasRemote;
  const pullDisabled = busy !== null || !st.upstream || st.behind === 0;
  const pushDisabled =
    busy !== null || !st.hasRemote || !st.branch || (!st.upstream && !st.published);
  const publishVisible = st.branch !== null && st.hasRemote && !st.upstream;

  const run = async (action: Action, fn: () => Promise<unknown>) => {
    setError(null);
    setBusy(action);
    try {
      await fn();
      void useChanges.getState().refetch(workspaceId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const label = (base: string, action: Action) => (busy === action ? "…" : base);

  return (
    <div className="git-footer">
      <div className="git-footer-row">
        <span className="git-branch" title={st.upstream ?? "no upstream"}>
          {st.branch ?? "(detached)"}
          {st.upstream && (
            <span className="git-counts">
              {st.ahead > 0 && <span className="git-ahead">↑{st.ahead}</span>}
              {st.behind > 0 && <span className="git-behind">↓{st.behind}</span>}
            </span>
          )}
        </span>
        <span className="git-footer-actions">
          <button
            className="git-footer-btn"
            disabled={fetchDisabled}
            title={st.hasRemote ? `Fetch from ${st.remote}` : "No remote configured"}
            onClick={() =>
              void run("fetch", () => useCommitState.getState().fetch(workspaceId))
            }
          >
            {label("Fetch", "fetch")}
          </button>
          <button
            className="git-footer-btn"
            disabled={pullDisabled}
            title={
              !st.upstream
                ? "No upstream configured"
                : st.behind === 0
                  ? "Up to date"
                  : `Fast-forward ${st.behind} commit${st.behind === 1 ? "" : "s"} from ${st.upstream}`
            }
            onClick={() =>
              void run("pull", () => useCommitState.getState().pull(workspaceId))
            }
          >
            {label("Pull", "pull")}
          </button>
          <button
            className="git-footer-btn"
            disabled={pushDisabled}
            title={
              !st.hasRemote
                ? "No remote configured"
                : !st.branch
                  ? "HEAD is detached"
                  : st.ahead > 0
                    ? `Push ${st.ahead} commit${st.ahead === 1 ? "" : "s"} to ${st.remote}`
                    : "Nothing to push"
            }
            onClick={() =>
              void run("push", () => useCommitState.getState().push(workspaceId))
            }
          >
            {label("Push", "push")}
          </button>
          {publishVisible && (
            <button
              className="git-footer-btn"
              disabled={busy !== null}
              title={`Push and set upstream on ${st.remote}`}
              onClick={() =>
                void run("publish", () => useCommitState.getState().publish(workspaceId))
              }
            >
              {label("Publish", "publish")}
            </button>
          )}
        </span>
      </div>
      {st.remotes.length === 0 && (
        <div className="git-footer-row git-add-remote">
          <input
            className="git-remote-name"
            placeholder="remote name"
            value={remoteName}
            disabled={busy === "add-remote"}
            onChange={(e) => setRemoteName(e.target.value)}
          />
          <input
            className="git-remote-url"
            placeholder="remote URL"
            value={remoteUrl}
            disabled={busy === "add-remote"}
            onChange={(e) => setRemoteUrl(e.target.value)}
          />
          <button
            className="git-footer-btn"
            disabled={busy !== null || remoteName.trim() === "" || remoteUrl.trim() === ""}
            title="Add this remote"
            onClick={() =>
              void run("add-remote", async () => {
                await useCommitState.getState().addRemote(workspaceId, remoteName, remoteUrl);
                setRemoteUrl("");
              })
            }
          >
            {label("Add remote", "add-remote")}
          </button>
        </div>
      )}
      {error && (
        <p className="git-footer-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
