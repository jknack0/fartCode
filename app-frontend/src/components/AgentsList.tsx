// 7d "Agents on this machine": shared detection list used by App settings
// and onboarding step two. Rows: installed (version · dir), update
// available (update ⌄ → hostDependencyUpdate), not found (install link →
// hostDependencyInstall), installing (live installer output tail). Tail
// line: registry counts.
//
// Install/update are gated by a confirm sheet naming the exact command and
// manager BEFORE anything runs — a `curl | bash` install executes remote
// code, so one click is not enough (#98).
import { useEffect, useState } from "react";
import { HostDependencyDto } from "../lib/tauri";
import { useDependencies } from "../store/dependencies";

/** Shorten a home-anchored path to ~ for the row meta. */
function tildify(p: string): string {
  return p.replace(/^\/(?:Users|home)\/[^/]+/, "~");
}

/** Directory of the detected binary ("~/.local/bin"). */
function binDir(path: string | null): string | null {
  if (!path) return null;
  const i = path.lastIndexOf("/");
  return i > 0 ? tildify(path.slice(0, i)) : tildify(path);
}

/** ANSI escape-sequence matcher (built from the ESC char so the regex
 * literal carries no control character — no-control-regex). */
const ANSI_ESCAPE = new RegExp(`${String.fromCharCode(27)}\\[[0-9;?]*[A-Za-z]`, "g");

/** Last installer output frame, ANSI-stripped. Installers redraw progress
 * bars with \r and color with ANSI escapes — take the last newline/CR
 * delimited frame so the row shows "fetching · 62%", not a wall of codes. */
function lastFrame(text: string): string | null {
  const frame = text.split(/[\r\n]+/).filter((l) => l.trim()).pop();
  if (!frame) return null;
  return frame.replace(ANSI_ESCAPE, "").trim();
}

type Pending = { dep: HostDependencyDto; action: "install" | "update" };

function AgentRow({
  dep,
  onInstall,
  onUpdate,
}: {
  dep: HostDependencyDto;
  onInstall: (dep: HostDependencyDto) => void;
  onUpdate: (dep: HostDependencyDto) => void;
}) {
  const installing = useDependencies((s) => !!s.installing[dep.providerId]);
  const progress = useDependencies((s) => s.progress[dep.providerId]);

  if (installing) {
    const frame = progress ? lastFrame(progress) : null;
    return (
      <li className="fc-agent-row">
        <span className="fc-agent-name">{dep.name}</span>
        <span className="fc-agent-meta fc-agent-installing">
          {frame ? `installing · ${frame}` : "installing"}
        </span>
      </li>
    );
  }

  if (!dep.installed) {
    return (
      <li className="fc-agent-row missing">
        <span className="fc-agent-name">{dep.name}</span>
        <span className="fc-agent-meta">
          not found
          {dep.installCommand && (
            <>
              {" · "}
              <button
                className="fc-agent-install"
                title={`Install ${dep.name} via ${dep.installManager ?? "its registry plan"}`}
                onClick={() => onInstall(dep)}
              >
                install
              </button>
            </>
          )}
        </span>
      </li>
    );
  }

  const updateAvailable = dep.latest !== null && dep.latest !== dep.version;
  const dir = binDir(dep.path);
  return (
    <li className="fc-agent-row">
      <span className="fc-agent-name">
        {dep.name}
        {dep.isDefault && <span className="fc-tag-green">default</span>}
      </span>
      {updateAvailable ? (
        <span className="fc-agent-meta">
          {dep.version ?? "?"} ·{" "}
          <button
            className="fc-agent-update"
            title={`Update ${dep.name} to ${dep.latest}`}
            onClick={() => onUpdate(dep)}
          >
            update ⌄
          </button>
        </span>
      ) : (
        <span className="fc-agent-meta">
          {dep.version ?? "?"}
          {dir ? ` · ${dir}` : ""}
        </span>
      )}
    </li>
  );
}

function InstallConfirm({ pending, onClose }: { pending: Pending; onClose: () => void }) {
  const install = useDependencies((s) => s.install);
  const update = useDependencies((s) => s.update);
  const { dep, action } = pending;
  const command = action === "install" ? dep.installCommand : dep.updateCommand;
  const manager = dep.installManager ?? "its registry manager";

  const confirm = () => {
    onClose();
    if (action === "install") void install(dep.providerId);
    else void update(dep.providerId);
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-confirm"
        role="dialog"
        aria-label={`Confirm ${action} ${dep.name}`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="fc-confirm-body">
          <div className="fc-confirm-title">
            {action === "install" ? "Install" : "Update"}{" "}
            <span className="fc-confirm-id">{dep.name}</span>?
          </div>
          <div className="fc-confirm-list">
            <div>
              runs via <span className="fc-confirm-id">{manager}</span>
            </div>
            <div className="fc-confirm-command">
              {command ?? "no command available"}
            </div>
          </div>
        </div>
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <button type="button" onClick={onClose}>
              <span className="fc-key">esc</span> cancel
            </button>
          </div>
          <div className="fc-modal-foot-side">
            <button type="button" onClick={confirm}>
              <span className="fc-key">↵</span> {action}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export default function AgentsList() {
  const deps = useDependencies((s) => s.deps);
  const summary = useDependencies((s) => s.summary);
  const loading = useDependencies((s) => s.loading);
  const error = useDependencies((s) => s.error);
  const load = useDependencies((s) => s.load);
  const [pending, setPending] = useState<Pending | null>(null);

  // Re-detect on every open — installs done outside the app must show up.
  useEffect(() => {
    void load(true);
  }, [load]);

  const more = summary ? Math.max(0, summary.total - deps.length) : null;

  return (
    <div className="fc-agents">
      <div className="fc-set-group-label">Detected</div>
      {error && <p className="fc-set-error">{error}</p>}
      {loading && deps.length === 0 ? (
        <p className="fc-agents-loading">detecting…</p>
      ) : (
        <ul className="fc-agent-list">
          {deps.map((d) => (
            <AgentRow
              key={d.providerId}
              dep={d}
              onInstall={(dep) => setPending({ dep, action: "install" })}
              onUpdate={(dep) => setPending({ dep, action: "update" })}
            />
          ))}
        </ul>
      )}
      {more !== null && summary && (
        <div className="fc-agents-tail">
          + {more} more in the registry · {summary.acp} acp
        </div>
      )}
      {pending && <InstallConfirm pending={pending} onClose={() => setPending(null)} />}
    </div>
  );
}
