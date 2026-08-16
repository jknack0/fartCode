// Changes-panel footer (restyled to design_handoff_v2 §5d): the everyday
// git verbs (fetch/pull/push/publish) left this footer for the ⌘K palette —
// what remains is the mono hint line (prefixed, when an upstream exists,
// by the ↑n ↓n <upstream> sync segment — #133), the add-remote mini-form for the
// no-remote edge (still driven by the shared CommitState DTO), and inline
// errors (the repo has no toast system).
import { useState } from "react";
import { useChanges } from "../store/changes";
import { useCommitState } from "../store/commit-state";
import { hint } from "../lib/useCommands";
import { useAsyncSubmit } from "../lib/useAsyncSubmit";
import { useUi } from "../store/ui";

export default function GitFooter({ workspaceId }: { workspaceId: string }) {
  const stateEntry = useCommitState((s) => s.byWorkspace[workspaceId]);
  // Re-render when bindings change so the palette chord in the hint stays live.
  useUi((s) => s.bindingsVersion);

  const { busy, error, run } = useAsyncSubmit();
  const [remoteName, setRemoteName] = useState("origin");
  const [remoteUrl, setRemoteUrl] = useState("");
  const st = stateEntry?.state ?? null;

  const addDisabled = busy || remoteName.trim() === "" || remoteUrl.trim() === "";
  const addRemote = async () => {
    if (addDisabled) return;
    await run(
      () => useCommitState.getState().addRemote(workspaceId, remoteName, remoteUrl),
      {
        onSuccess: () => {
          setRemoteUrl("");
          void useChanges.getState().refetch(workspaceId);
        },
      },
    );
  };

  const paletteKey = hint("open-command-palette") || "⌘K";

  return (
    <div className="fc-git-footer">
      {st && st.remotes.length === 0 && (
        <div className="fc-add-remote">
          <input
            className="fc-remote-name"
            aria-label="Remote name"
            placeholder="remote"
            value={remoteName}
            disabled={busy}
            onChange={(e) => setRemoteName(e.target.value)}
          />
          <input
            className="fc-remote-url"
            aria-label="Remote URL"
            placeholder="remote URL"
            value={remoteUrl}
            disabled={busy}
            onChange={(e) => setRemoteUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                void addRemote();
              }
            }}
          />
          <span
            role="button"
            className="fc-add-remote-btn"
            aria-disabled={addDisabled}
            title="Add this remote"
            onClick={() => void addRemote()}
          >
            {busy ? "…" : "add"}
          </span>
        </div>
      )}
      {error && (
        <p className="fc-footer-error" role="alert">
          {error}
        </p>
      )}
      <p className="fc-footer-hint">
        {st?.upstream != null && `↑${st.ahead} ↓${st.behind} ${st.upstream} · `}
        d discards after a confirm · fetch / pull / push in {paletteKey}
      </p>
    </div>
  );
}
