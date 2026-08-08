// Pull Requests tab (E4-07, #47): renders the cached PR for the workspace's
// branch — merge state, files, commits, checks (failed first), comments —
// with the empty states (no token / not GitHub / no PR) and token import.
import { useEffect, useState } from "react";
import {
  githubTokenImport,
  githubTokenSet,
  githubTokenStatus,
  type GithubTokenStatusDto,
} from "../lib/tauri";
import { usePr } from "../store/pr";
import { IconGitHub } from "./icons";

const STATUS_LABEL: Record<string, string> = {
  open: "Open",
  closed: "Closed",
  merged: "Merged",
};

const MERGE_STATE_LABEL: Record<string, string> = {
  clean: "Mergeable",
  dirty: "Conflicts",
  blocked: "Blocked",
  behind: "Behind base",
  unstable: "Checks failing",
  has_hooks: "Has hooks",
  unknown: "Checking…",
};

function checkGlyph(status: string, conclusion: string | null): string {
  if (status !== "completed") return "◌";
  switch (conclusion) {
    case "success":
      return "✓";
    case "failure":
    case "timed_out":
    case "startup_failure":
      return "✗";
    case "cancelled":
      return "⊘";
    case "skipped":
      return "↷";
    default:
      return "•";
  }
}

function durationOf(startedAt: string | null, completedAt: string | null): string | null {
  if (!startedAt || !completedAt) return null;
  const ms = Date.parse(completedAt) - Date.parse(startedAt);
  if (!Number.isFinite(ms) || ms < 0) return null;
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

export default function PullRequestPanel({ workspaceId }: { workspaceId: string }) {
  const entry = usePr((s) => s.byWorkspace[workspaceId]);

  useEffect(() => {
    void usePr.getState().ensure(workspaceId);
  }, [workspaceId]);

  const section = entry?.section ?? null;

  if (entry?.loading && !section) {
    return <p className="changes-empty muted">Loading pull requests…</p>;
  }
  if (!section) {
    return (
      <div className="changes-empty">
        <p className="error">{entry?.error ?? "Unknown error"}</p>
        <button onClick={() => void usePr.getState().refetch(workspaceId)}>Retry</button>
      </div>
    );
  }
  if (!section.tokenPresent) return <TokenGate workspaceId={workspaceId} />;
  if (!section.repository) {
    return (
      <p className="changes-empty muted">
        No GitHub remote on this branch — push to GitHub to see pull requests.
      </p>
    );
  }
  if (!section.pr) {
    return (
      <div className="pr-empty">
        <p className="changes-empty muted">
          No open pull request for <code>{section.branch}</code>.
        </p>
        <button
          className="pr-refresh"
          title="Sync with GitHub now"
          onClick={() => void usePr.getState().sync(workspaceId)}
          disabled={entry?.loading}
        >
          {entry?.loading ? "Syncing…" : "Check again"}
        </button>
      </div>
    );
  }

  const pr = section.pr;
  return (
    <div className="pr-body">
      <div className="pr-header">
        <a className="pr-title" href={pr.url} target="_blank" rel="noreferrer">
          <IconGitHub size={12} />
          <span>
            {pr.title} <span className="pr-number">#{pr.number}</span>
          </span>
        </a>
        <div className="pr-badges">
          <span className={`pr-badge status-${pr.status}`}>
            {STATUS_LABEL[pr.status] ?? pr.status}
          </span>
          {pr.draft && <span className="pr-badge draft">Draft</span>}
          <span className="pr-badge merge">
            {pr.mergeableState
              ? (MERGE_STATE_LABEL[pr.mergeableState] ?? pr.mergeableState)
              : "Merge state pending"}
          </span>
        </div>
        <div className="pr-refs muted">
          <code>{pr.headRef}</code> → <code>{pr.baseRef}</code>
          <span className="pr-stats">
            <em className="add">+{pr.additions}</em> <em className="del">−{pr.deletions}</em> ·{" "}
            {pr.changedFiles} files
          </span>
        </div>
        {entry?.loading && <span className="muted pr-syncing">Syncing…</span>}
      </div>

      <section className="changes-section">
        <h3>
          Checks<span className="changes-count">{pr.checks.length}</span>
        </h3>
        {pr.checks.length === 0 ? (
          <p className="changes-empty muted">No checks reported.</p>
        ) : (
          <ul>
            {pr.checks.map((check) => (
              <li key={check.id} className={`pr-check conclusion-${check.conclusion ?? "pending"}`}>
                <span className="pr-check-glyph">{checkGlyph(check.status, check.conclusion)}</span>
                {check.url ? (
                  <a href={check.url} target="_blank" rel="noreferrer">
                    {check.name}
                  </a>
                ) : (
                  <span>{check.name}</span>
                )}
                <span className="pr-check-meta muted">
                  {check.appName ? `${check.appName} · ` : ""}
                  {durationOf(check.startedAt, check.completedAt) ?? check.status}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="changes-section">
        <h3>
          Files<span className="changes-count">{pr.files.length}</span>
        </h3>
        <ul>
          {pr.files.map((file) => (
            <li key={file.filename} className={`change-row status-${fileGlyph(file.status)}`}>
              <span className="change-glyph">{fileGlyphLetter(file.status)}</span>
              <span className="change-path">{file.filename}</span>
              <span className="change-stats">
                <em className="add">+{file.additions}</em>
                <em className="del">−{file.deletions}</em>
              </span>
            </li>
          ))}
        </ul>
      </section>

      <section className="changes-section">
        <h3>
          Commits<span className="changes-count">{pr.commits.length}</span>
        </h3>
        <ul>
          {pr.commits.map((commit) => (
            <li key={commit.sha} className="pr-commit">
              {commit.url ? (
                <a href={commit.url} target="_blank" rel="noreferrer" title={commit.sha}>
                  {commit.subject || commit.sha.slice(0, 8)}
                </a>
              ) : (
                <span title={commit.sha}>{commit.subject || commit.sha.slice(0, 8)}</span>
              )}
              <span className="pr-commit-meta muted">
                {commit.author?.login ?? commit.authorName ?? "?"}
                {commit.date ? ` · ${new Date(commit.date).toLocaleDateString()}` : ""}
              </span>
            </li>
          ))}
        </ul>
      </section>

      <section className="changes-section">
        <h3>
          Comments<span className="changes-count">{pr.comments.length}</span>
        </h3>
        {pr.comments.length === 0 ? (
          <p className="changes-empty muted">No comments yet.</p>
        ) : (
          <ul>
            {pr.comments.map((comment) => (
              <li key={`${comment.kind}-${comment.id}`} className="pr-comment">
                <div className="pr-comment-head">
                  <strong>{comment.author?.login ?? "unknown"}</strong>
                  {comment.kind === "review" && comment.path && (
                    <code className="muted">
                      {comment.path}
                      {comment.line ? `:${comment.line}` : ""}
                    </code>
                  )}
                  <span className="muted">
                    {comment.createdAt ? new Date(comment.createdAt).toLocaleDateString() : ""}
                  </span>
                </div>
                {comment.url ? (
                  <a
                    className="pr-comment-body"
                    href={comment.url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {comment.body}
                  </a>
                ) : (
                  <span className="pr-comment-body">{comment.body}</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <div className="pr-refresh-row">
        <button
          className="pr-refresh"
          onClick={() => void usePr.getState().sync(workspaceId)}
          disabled={entry?.loading}
        >
          {entry?.loading ? "Syncing…" : "Sync now"}
        </button>
      </div>
    </div>
  );
}

function fileGlyph(status: string): string {
  switch (status) {
    case "added":
      return "added";
    case "removed":
      return "deleted";
    case "renamed":
      return "renamed";
    default:
      return "modified";
  }
}

function fileGlyphLetter(status: string): string {
  switch (status) {
    case "added":
      return "A";
    case "removed":
      return "D";
    case "renamed":
      return "R";
    default:
      return "M";
  }
}

// Empty-state gate when no GitHub token is configured: import from the gh
// CLI or paste a PAT. Tokens only ever go to the OS keyring.
function TokenGate({ workspaceId }: { workspaceId: string }) {
  const [status, setStatus] = useState<GithubTokenStatusDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pasting, setPasting] = useState(false);
  const [tokenInput, setTokenInput] = useState("");

  useEffect(() => {
    void githubTokenStatus().then(setStatus).catch(() => {});
  }, []);

  const refresh = () =>
    githubTokenStatus()
      .then((s) => {
        setStatus(s);
        if (s.connected) void usePr.getState().sync(workspaceId);
      })
      .catch((e) => setError(String(e)));

  return (
    <div className="pr-token-gate">
      <p className="changes-empty muted">
        Connect GitHub to see pull requests. The token lives in your OS keyring —
        never in the app database.
      </p>
      {status?.connected && <p className="muted">Token stored ({status.mask}).</p>}
      <div className="pr-token-actions">
        <button
          className="primary"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            setError(null);
            try {
              await githubTokenImport();
              refresh();
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy ? "Importing…" : "Import from gh CLI"}
        </button>
        <button onClick={() => setPasting((p) => !p)}>
          {pasting ? "Cancel" : "Paste token"}
        </button>
      </div>
      {pasting && (
        <div className="pr-token-paste">
          <input
            type="password"
            placeholder="ghp_… / github_pat_…"
            value={tokenInput}
            onChange={(e) => setTokenInput(e.target.value)}
          />
          <button
            className="primary"
            disabled={!tokenInput.trim() || busy}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await githubTokenSet(tokenInput.trim());
                setTokenInput("");
                setPasting(false);
                refresh();
              } catch (e) {
                setError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            Save
          </button>
        </div>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
