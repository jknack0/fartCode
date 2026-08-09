// PR & checks tab (E4-07, #47 — restyled per design_handoff_v2 §5e):
// 46px header (`pr N` + head → base), mono tab row files/commits/checks/
// comments with accent underline, check rows failed-first with 7px status
// dots, and a mono footer (merge state left, `r refresh` right — r kicks
// the sync engine while the panel has focus). Data still comes cache-first
// from the pr store (`pr_section_get` + background sync, `pr:updated`
// events); the empty states (no token / not GitHub / no PR) are preserved.
import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import {
  githubTokenImport,
  githubTokenSet,
  githubTokenStatus,
  type GithubTokenStatusDto,
  type PrCheckDto,
  type PrDto,
} from "../lib/tauri";
import { usePr } from "../store/pr";

type CheckKind = "failed" | "running" | "passed" | "queued" | "other";

const FAILED_CONCLUSIONS = new Set(["failure", "timed_out", "startup_failure"]);
const QUEUED_STATUSES = new Set(["queued", "pending", "waiting", "requested"]);

function checkKind(check: PrCheckDto): CheckKind {
  if (check.status === "completed") {
    if (check.conclusion === "success") return "passed";
    if (check.conclusion && FAILED_CONCLUSIONS.has(check.conclusion)) return "failed";
    return "other";
  }
  if (check.status === "in_progress") return "running";
  if (QUEUED_STATUSES.has(check.status)) return "queued";
  return "other";
}

/** Failed sorts to top, then running, passed, the rest; queued sinks. */
const KIND_RANK: Record<CheckKind, number> = {
  failed: 0,
  running: 1,
  passed: 2,
  other: 3,
  queued: 4,
};

const DOT_CLASS: Record<CheckKind, string> = {
  failed: "status-failed",
  running: "status-in_progress",
  passed: "fc-pr-dot-passed",
  queued: "fc-pr-dot-queued",
  other: "fc-pr-dot-idle",
};

/** "41s" / "2m 14s" / "1m 02s" — completed-run duration. */
function durationOf(startedAt: string | null, completedAt: string | null): string | null {
  if (!startedAt || !completedAt) return null;
  const ms = Date.parse(completedAt) - Date.parse(startedAt);
  if (!Number.isFinite(ms) || ms < 0) return null;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

/** Coarse elapsed for a still-running check: "38s" / "1m". */
function elapsedOf(startedAt: string | null): string | null {
  if (!startedAt) return null;
  const ms = Date.now() - Date.parse(startedAt);
  if (!Number.isFinite(ms) || ms < 0) return null;
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m`;
}

/** Footer merge-state words when nothing is failing (lowercase, quiet). */
const MERGE_LABEL: Record<string, string> = {
  clean: "mergeable",
  dirty: "conflicts",
  blocked: "blocked",
  behind: "behind base",
  unstable: "checks failing",
  has_hooks: "has hooks",
  unknown: "checking…",
};

function mergeLabel(pr: PrDto): string {
  if (pr.status === "merged") return "merged";
  if (pr.status === "closed") return "closed";
  const state = pr.mergeableState
    ? (MERGE_LABEL[pr.mergeableState] ?? pr.mergeableState)
    : "checking…";
  return pr.draft ? `draft · ${state}` : state;
}

const FILE_LETTER: Record<string, string> = {
  added: "A",
  removed: "D",
  renamed: "R",
  modified: "M",
};

type PrTab = "files" | "commits" | "checks" | "comments";

export default function PullRequestPanel({ workspaceId }: { workspaceId: string }) {
  const entry = usePr((s) => s.byWorkspace[workspaceId]);
  const [tab, setTab] = useState<PrTab>("checks");

  useEffect(() => {
    void usePr.getState().ensure(workspaceId);
  }, [workspaceId]);

  // `r` = refresh while the panel has focus (design 5e footer key). The
  // sync store call is the same one the old "Sync now" button ran.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "r" || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const t = e.target as HTMLElement;
    if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
    e.preventDefault();
    void usePr.getState().sync(workspaceId);
  };

  const section = entry?.section ?? null;

  let inner: ReactNode;
  if (entry?.loading && !section) {
    inner = <div className="fc-pr-state">loading pull request…</div>;
  } else if (!section) {
    inner = (
      <div className="fc-pr-state">
        <p className="error">{entry?.error ?? "Unknown error"}</p>
        <button onClick={() => void usePr.getState().refetch(workspaceId)}>Retry</button>
      </div>
    );
  } else if (!section.tokenPresent) {
    inner = <TokenGate workspaceId={workspaceId} />;
  } else if (!section.repository) {
    inner = (
      <div className="fc-pr-state">
        no GitHub remote on this branch — push to GitHub to see pull requests
      </div>
    );
  } else if (!section.pr) {
    inner = (
      <>
        <div className="fc-pr-state">
          no open pull request for <span className="fc-pr-state-branch">{section.branch}</span>
        </div>
        <PanelFooter left={null} loading={entry?.loading ?? false} workspaceId={workspaceId} />
      </>
    );
  } else {
    inner = <PrBody pr={section.pr} loading={entry?.loading ?? false} workspaceId={workspaceId} tab={tab} setTab={setTab} />;
  }

  return (
    <div className="fc-pr" tabIndex={0} onKeyDown={onKeyDown}>
      {inner}
    </div>
  );
}

function PrBody({
  pr,
  loading,
  workspaceId,
  tab,
  setTab,
}: {
  pr: PrDto;
  loading: boolean;
  workspaceId: string;
  tab: PrTab;
  setTab: (t: PrTab) => void;
}) {
  const checks = [...pr.checks].sort((a, b) => KIND_RANK[checkKind(a)] - KIND_RANK[checkKind(b)]);
  const failing = checks.filter((c) => checkKind(c) === "failed").length;
  const anyRunning = checks.some((c) => checkKind(c) === "running");

  // While any check is running, re-render on a coarse 30s tick so the
  // `elapsedOf` meta ("running · 38s") doesn't go stale between syncs.
  // The counter exists only to force the render; cleared when nothing runs.
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!anyRunning) return;
    const id = window.setInterval(() => setTick((t) => t + 1), 30_000);
    return () => window.clearInterval(id);
  }, [anyRunning]);

  const tabs: { id: PrTab; label: string; count: number }[] = [
    { id: "files", label: "files", count: pr.changedFiles },
    { id: "commits", label: "commits", count: pr.commitCount },
    { id: "checks", label: "checks", count: pr.checks.length },
    { id: "comments", label: "comments", count: pr.comments.length },
  ];

  return (
    <>
      <header className="fc-pr-head">
        <a
          className="fc-pr-id"
          href={pr.url}
          target="_blank"
          rel="noreferrer"
          title={pr.title}
        >
          pr {pr.number}
        </a>
        <span className="fc-pr-refs" title={`${pr.headRef} → ${pr.baseRef}`}>
          {pr.headRef} → {pr.baseRef}
        </span>
      </header>

      <nav className="fc-pr-tabs">
        {tabs.map((t) => (
          <button
            key={t.id}
            className={tab === t.id ? "active" : ""}
            onClick={() => setTab(t.id)}
          >
            {t.label} {t.count}
          </button>
        ))}
      </nav>

      <div className="fc-pr-body">
        {tab === "checks" &&
          (checks.length === 0 ? (
            <div className="fc-pr-empty">no checks reported</div>
          ) : (
            <ul className="fc-pr-list">
              {checks.map((check) => (
                <CheckRow key={check.id} check={check} />
              ))}
            </ul>
          ))}

        {tab === "files" &&
          (pr.files.length === 0 ? (
            <div className="fc-pr-empty">no files</div>
          ) : (
            <ul className="fc-pr-list dense">
              {pr.files.map((file) => (
                <li key={file.filename} className="fc-pr-file">
                  <span className="fc-pr-file-letter">
                    {FILE_LETTER[file.status] ?? "M"}
                  </span>
                  <span className="fc-pr-file-path" title={file.filename}>
                    {file.filename}
                  </span>
                  <span className="fc-pr-file-stats">
                    +{file.additions} −{file.deletions}
                  </span>
                </li>
              ))}
            </ul>
          ))}

        {tab === "commits" &&
          (pr.commits.length === 0 ? (
            <div className="fc-pr-empty">no commits</div>
          ) : (
            <ul className="fc-pr-list">
              {pr.commits.map((commit) => (
                <li key={commit.sha} className="fc-pr-commit">
                  <div className="fc-pr-commit-subject" title={commit.sha}>
                    {commit.url ? (
                      <a href={commit.url} target="_blank" rel="noreferrer">
                        {commit.subject || commit.sha.slice(0, 8)}
                      </a>
                    ) : (
                      commit.subject || commit.sha.slice(0, 8)
                    )}
                  </div>
                  <div className="fc-pr-commit-meta">
                    {commit.sha.slice(0, 8)} ·{" "}
                    {commit.author?.login ?? commit.authorName ?? "?"}
                    {commit.date ? ` · ${new Date(commit.date).toLocaleDateString()}` : ""}
                  </div>
                </li>
              ))}
            </ul>
          ))}

        {tab === "comments" &&
          (pr.comments.length === 0 ? (
            <div className="fc-pr-empty">no comments yet</div>
          ) : (
            <ul className="fc-pr-list">
              {pr.comments.map((comment) => (
                <li key={`${comment.kind}-${comment.id}`} className="fc-pr-comment">
                  <div className="fc-pr-comment-head">
                    <span className="fc-pr-comment-author">
                      {comment.author?.login ?? "unknown"}
                    </span>
                    {comment.kind === "review" && comment.path && (
                      <span className="fc-pr-comment-path">
                        {comment.path}
                        {comment.line ? `:${comment.line}` : ""}
                      </span>
                    )}
                    {comment.createdAt && (
                      <span>{new Date(comment.createdAt).toLocaleDateString()}</span>
                    )}
                  </div>
                  {comment.url ? (
                    <a
                      className="fc-pr-comment-body"
                      href={comment.url}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {comment.body}
                    </a>
                  ) : (
                    <span className="fc-pr-comment-body">{comment.body}</span>
                  )}
                </li>
              ))}
            </ul>
          ))}
      </div>

      <PanelFooter
        left={
          failing > 0 ? (
            <span className="fc-pr-foot-bad">
              {failing} failing — merge blocked
            </span>
          ) : (
            <span>{mergeLabel(pr)}</span>
          )
        }
        loading={loading}
        workspaceId={workspaceId}
      />
    </>
  );
}

function CheckRow({ check }: { check: PrCheckDto }) {
  const kind = checkKind(check);

  let meta: string;
  switch (kind) {
    case "failed": {
      const dur = durationOf(check.startedAt, check.completedAt);
      meta = dur ? `failed · ${dur}` : "failed";
      break;
    }
    case "running": {
      const elapsed = elapsedOf(check.startedAt);
      meta = elapsed ? `running · ${elapsed}` : "running";
      break;
    }
    case "passed": {
      const dur = durationOf(check.startedAt, check.completedAt);
      meta = dur ? `passed · ${dur}` : "passed";
      break;
    }
    case "queued":
      meta = "queued";
      break;
    default: {
      const word = check.conclusion ?? check.status;
      const dur = durationOf(check.startedAt, check.completedAt);
      meta = dur ? `${word} · ${dur}` : word;
    }
  }

  return (
    <li className={`fc-pr-check ${kind}`}>
      <span className={`status-dot fc-pr-dot ${DOT_CLASS[kind]}`} />
      <div className="fc-pr-check-main">
        <div className="fc-pr-check-name" title={check.appName ?? undefined}>
          {check.url ? (
            <a href={check.url} target="_blank" rel="noreferrer">
              {check.name}
            </a>
          ) : (
            check.name
          )}
        </div>
        <div className="fc-pr-check-meta">
          {meta}
          {kind === "failed" && check.url && (
            <>
              {" · "}
              <a href={check.url} target="_blank" rel="noreferrer">
                logs
              </a>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

// Footer above a hairline: merge state left, `r refresh` right. No registry
// command exists for PR refresh, so the key label is the literal `r`.
function PanelFooter({
  left,
  loading,
  workspaceId,
}: {
  left: ReactNode;
  loading: boolean;
  workspaceId: string;
}) {
  return (
    <footer className="fc-pr-foot">
      <span className="fc-pr-foot-left">{left}</span>
      <button
        className="fc-pr-refresh"
        title="Sync with GitHub now"
        disabled={loading}
        onClick={() => void usePr.getState().sync(workspaceId)}
      >
        {loading ? "syncing…" : (
          <>
            <kbd>r</kbd> refresh
          </>
        )}
      </button>
    </footer>
  );
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
    <div className="fc-pr-state fc-pr-token-gate">
      <p>
        Connect GitHub to see pull requests. The token lives in your OS keyring
        — never in the app database.
      </p>
      {status?.connected && <p>Token stored ({status.mask}).</p>}
      <div className="fc-pr-token-actions">
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
        <div className="fc-pr-token-paste">
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
