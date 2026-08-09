// 7d "Agents on this machine": shared detection list used by App settings
// and onboarding step two. Rows: installed (version · dir), update
// available (update ⌄ → hostDependencyUpdate), not found (install link →
// hostDependencyInstall), installing (bare — the runner emits no progress
// events). Tail line: registry counts.
import { useEffect } from "react";
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

function AgentRow({ dep }: { dep: HostDependencyDto }) {
  const installing = useDependencies((s) => !!s.installing[dep.providerId]);
  const install = useDependencies((s) => s.install);
  const update = useDependencies((s) => s.update);

  if (installing) {
    return (
      <li className="fc-agent-row">
        <span className="fc-agent-name">{dep.name}</span>
        <span className="fc-agent-meta fc-agent-installing">installing</span>
      </li>
    );
  }

  if (!dep.installed) {
    return (
      <li className="fc-agent-row missing">
        <span className="fc-agent-name">{dep.name}</span>
        <span className="fc-agent-meta">
          not found ·{" "}
          <button
            className="fc-agent-install"
            title={`Install ${dep.name} via its registry plan`}
            onClick={() => void install(dep.providerId)}
          >
            install
          </button>
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
            onClick={() => void update(dep.providerId)}
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

export default function AgentsList() {
  const deps = useDependencies((s) => s.deps);
  const summary = useDependencies((s) => s.summary);
  const loading = useDependencies((s) => s.loading);
  const error = useDependencies((s) => s.error);
  const load = useDependencies((s) => s.load);

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
            <AgentRow key={d.providerId} dep={d} />
          ))}
        </ul>
      )}
      {more !== null && summary && (
        <div className="fc-agents-tail">
          + {more} more in the registry · {summary.acp} acp
        </div>
      )}
    </div>
  );
}
