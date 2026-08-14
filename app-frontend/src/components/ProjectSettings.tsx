// Project settings pane (E1-05, design_handoff_v2 7c): the full field map
// as boxless rows in three groups (Repo / Workspaces / Agent). Every row
// opens its editor inline underneath; edits save immediately through
// updateProjectSettings. Shareable keys (preservePatterns / shellSetup /
// scripts) carry a green `shared` tag when the .fartCode.json value wins
// (projectSettingsProvenance); ⇧⌘S moves local values into .fartCode.json
// (projectSettingsShare, E1-02) — a modal-local chord, not a registry
// command. The default export renders the unified settings surface
// (SettingsModal shell) opened at this project, for the ⌘-gear path.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  DefaultBranchDto,
  ProjectSettingsDto,
  TaskDto,
  getProjectSettings,
  listTasks,
  onFartcodeEvent,
  projectSettingsProvenance,
  projectSettingsShare,
  remoteTasksEnabled,
  restoreTask,
  setDefaultAgent,
  updateProjectSettings,
} from "../lib/tauri";
import { wireEvents } from "../lib/wireEvents";
import { defaultAgentName, useDependencies } from "../store/dependencies";
import { useUi } from "../store/ui";
import SettingsModal from "./SettingsModal";

const DEFAULTS = {
  worktreeDirectory: "~/fartCode/worktrees",
  defaultBranch: "main",
  baseRemote: "origin",
};

type Provenance = Record<string, "default" | "shared" | "local">;

const branchName = (b?: DefaultBranchDto | null): string =>
  typeof b === "string" ? b : b?.name ?? "";

/* -- row shell: label 13px left · value mono 11px right (⌄ = menu) ------- */

function Row({
  label,
  value,
  shared,
  chevron,
  open,
  onClick,
  children,
}: {
  label: string;
  value: string;
  shared?: boolean;
  chevron?: boolean;
  open?: boolean;
  onClick?: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div className={`fc-set-row-wrap${open ? " open" : ""}`}>
      <button
        type="button"
        className={`fc-set-row${onClick ? "" : " static"}`}
        onClick={onClick}
        tabIndex={onClick ? 0 : -1}
      >
        <span className="fc-set-label">
          {label}
          {shared && <span className="fc-tag-green">shared</span>}
        </span>
        <span className="fc-set-value">
          {value}
          {chevron ? " ⌄" : ""}
        </span>
      </button>
      {open && children}
    </div>
  );
}

/* -- inline text editor: ↵ save · esc cancel ----------------------------- */

function InlineInput({
  initial,
  placeholder,
  onSave,
  onCancel,
}: {
  initial: string;
  placeholder?: string;
  onSave: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  return (
    <div className="fc-set-editor">
      <input
        autoFocus
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onSave(v);
          } else if (e.key === "Escape") {
            // Keep esc local — it must not close the whole settings surface.
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        onBlur={() => onSave(v)}
      />
      <div className="fc-set-editor-keys">↵ save · esc cancel</div>
    </div>
  );
}

function InlineTextarea({
  initial,
  placeholder,
  rows = 3,
  onSave,
  onCancel,
}: {
  initial: string;
  placeholder?: string;
  rows?: number;
  onSave: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(initial);
  return (
    <div className="fc-set-editor">
      <textarea
        autoFocus
        rows={rows}
        value={v}
        placeholder={placeholder}
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            onSave(v);
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        onBlur={() => onSave(v)}
      />
      <div className="fc-set-editor-keys">⌘↵ save · esc cancel</div>
    </div>
  );
}

/* -- the pane ------------------------------------------------------------ */

export function ProjectSettingsPane({ projectId }: { projectId: string }) {
  const [s, setS] = useState<ProjectSettingsDto>({});
  const [prov, setProv] = useState<Provenance>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [openRow, setOpenRow] = useState<string | null>(null);
  const deps = useDependencies((st) => st.deps);
  const loadDeps = useDependencies((st) => st.load);
  const noticeTimer = useRef<number | null>(null);
  /** Serializes commits — see `commit`. */
  const writeChain = useRef<Promise<void>>(Promise.resolve());
  /** Latest settings for the chained closures, which would otherwise
   * capture the `s` of the render that queued them. */
  const sRef = useRef<ProjectSettingsDto>({});
  sRef.current = s;

  // The Agent group names the default agent — serve the 300s detection
  // cache if the App pane hasn't populated the store yet.
  useEffect(() => {
    if (deps.length === 0) void loadDeps(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // `setting:changed` (key `defaultAgent`) — refetch so `isDefault` flips
  // even when the setting was changed elsewhere. Cache-served: is_default
  // is recomputed from settings on every list call.
  useEffect(
    () =>
      wireEvents(onFartcodeEvent, (ev) => {
        if (ev.type === "setting:changed" && ev.key === "defaultAgent") {
          void loadDeps(false);
        }
      }),
    [loadDeps],
  );

  const refresh = useCallback(async () => {
    const [settings, provenance] = await Promise.all([
      getProjectSettings(projectId),
      projectSettingsProvenance(projectId).catch(() => ({}) as Provenance),
    ]);
    setS(settings);
    setProv(provenance);
  }, [projectId]);

  // E12-10 gate: the provision/terminate row only renders in builds compiled
  // with `remote-tasks` — configuring scripts a build cannot run is a trap.
  const [byoiEnabled, setByoiEnabled] = useState(false);
  useEffect(() => {
    remoteTasksEnabled()
      .then(setByoiEnabled)
      .catch(() => setByoiEnabled(false));
  }, []);

  // #136: archived tasks were enumerable nowhere — this section is the
  // recovery surface. list_tasks returns every row (the board and sidebar
  // filter archivedAt client-side), so archived rows only ever show here.
  const [archived, setArchived] = useState<TaskDto[]>([]);
  const loadArchived = useCallback(
    () =>
      listTasks(projectId)
        .then((ts) => setArchived(ts.filter((t) => t.archivedAt !== null)))
        .catch(() => {}),
    [projectId],
  );
  useEffect(() => {
    void loadArchived();
    return wireEvents(onFartcodeEvent, (ev) => {
      if (
        ev.type === "task:archived" ||
        ev.type === "task:restored" ||
        ev.type === "task:deleted"
      ) {
        void loadArchived();
      }
    });
  }, [loadArchived]);

  useEffect(() => {
    setLoading(true);
    setOpenRow(null);
    setError(null);
    refresh()
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [refresh]);

  useEffect(
    () => () => {
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const flash = (text: string) => {
    setNotice(text);
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), 4000);
  };

  /** Immediate persistence: optimistic local state, then the saved echo +
   * fresh provenance (a local edit to a shared key flips its tag).
   *
   * **The write merges onto a FRESH read, not onto this pane's copy.**
   * `update_project_settings` is full-replace, and `s` is a snapshot taken
   * when the pane mounted — so a pane left open across a
   * `featureDossiers` answer or a `featureLogSeededVersion` bump would
   * send the pre-change values back and silently undo them. Toggling
   * `tmux` must not un-answer the dossier consent card or make the app
   * forget a scaffold it already seeded. The patch still wins over the
   * fresh read: it is this pane's actual edit.
   *
   * **And the read-write pair is serialized.** Two quick toggles would
   * otherwise interleave — B's read starts before A's write commits, so B
   * reads the pre-A row and its full-replace write silently undoes A.
   * Chaining on a ref keeps every commit's read strictly after the
   * previous commit's write. */
  const commit = (patch: Partial<ProjectSettingsDto>): Promise<void> => {
    setS((cur) => ({ ...cur, ...patch }));
    setOpenRow(null);
    setError(null);
    const run = writeChain.current
      .then(async () => {
        const fresh = await getProjectSettings(projectId).catch(() => sRef.current);
        const saved = await updateProjectSettings(projectId, { ...fresh, ...patch });
        setS(saved);
        setProv(await projectSettingsProvenance(projectId).catch(() => prov));
      })
      .catch((e) => setError(String(e)));
    writeChain.current = run;
    return run;
  };

  /** Default-agent pick: app-wide setting, then a cache-served refetch so
   * the green `default` tag flips (the event listener covers other panes). */
  const chooseAgent = async (providerId: string) => {
    setOpenRow(null);
    setError(null);
    try {
      await setDefaultAgent(providerId);
      await loadDeps(false);
    } catch (e) {
      setError(String(e));
    }
  };

  // ⇧⌘S — share local values into .fartCode.json, then refresh provenance.
  // Modal-local (capture phase) so it wins over the registry dispatcher.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.shiftKey && !e.altKey && !e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        e.stopPropagation();
        void projectSettingsShare(projectId)
          .then(refresh)
          .then(() => flash("local values moved into .fartCode.json"))
          .catch((err) => setError(String(err)));
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId, refresh]);

  if (loading) return <div className="fc-set-loading">loading…</div>;

  const toggle = (row: string) => setOpenRow((r) => (r === row ? null : row));
  const sharedTag = (key: string) => prov[key] === "shared";

  const base = s.baseRemote || DEFAULTS.baseRemote;
  const push = s.pushRemote || base;
  const patterns = s.preservePatterns ?? [];
  const scriptMark = (v?: string | null) => (v && v.trim() ? "✓" : "—");
  const scriptsSummary = `setup ${scriptMark(s.scripts?.setup)} · run ${scriptMark(
    s.scripts?.run,
  )} · teardown ${scriptMark(s.scripts?.teardown)}`;

  return (
    <div className="fc-set-pane-body">
      {error && <p className="fc-set-error">{error}</p>}

      <div className="fc-set-group">
        <div className="fc-set-group-label">Repo</div>
        <Row
          label="Default branch"
          value={branchName(s.defaultBranch) || DEFAULTS.defaultBranch}
          chevron
          open={openRow === "defaultBranch"}
          onClick={() => toggle("defaultBranch")}
        >
          <InlineInput
            initial={branchName(s.defaultBranch)}
            placeholder={DEFAULTS.defaultBranch}
            onCancel={() => setOpenRow(null)}
            onSave={(v) => {
              const value = v.trim() || null;
              // Preserve remote-tracking when the loaded branch is {name, remote: true}.
              if (
                value &&
                typeof s.defaultBranch === "object" &&
                s.defaultBranch !== null &&
                s.defaultBranch.remote
              ) {
                void commit({ defaultBranch: { name: value, remote: true } });
              } else {
                void commit({ defaultBranch: value });
              }
            }}
          />
        </Row>
        <Row
          label="Base remote · push remote"
          value={`${base} · ${push}`}
          chevron
          open={openRow === "remotes"}
          onClick={() => toggle("remotes")}
        >
          <div className="fc-set-editor fc-set-editor-pair">
            <input
              autoFocus
              defaultValue={s.baseRemote ?? ""}
              placeholder={DEFAULTS.baseRemote}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setOpenRow(null);
                }
              }}
              onBlur={(e) => {
                const v = e.target.value.trim() || null;
                if (v !== (s.baseRemote ?? null)) void commit({ baseRemote: v });
              }}
            />
            <input
              defaultValue={s.pushRemote ?? ""}
              placeholder="same as base"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setOpenRow(null);
                }
              }}
              onBlur={(e) => {
                const v = e.target.value.trim() || null;
                if (v !== (s.pushRemote ?? null)) void commit({ pushRemote: v });
              }}
            />
            <div className="fc-set-editor-keys">base · push — saves on blur · esc closes</div>
          </div>
        </Row>
        <Row
          label="GitHub account"
          value={s.githubAccountId || "—"}
          chevron
          open={openRow === "github"}
          onClick={() => toggle("github")}
        >
          <InlineInput
            initial={s.githubAccountId ?? ""}
            placeholder="account id"
            onCancel={() => setOpenRow(null)}
            onSave={(v) => void commit({ githubAccountId: v.trim() || null })}
          />
        </Row>
        <Row
          label="Import GitHub issues on open"
          value={s.autoImport !== false ? "on" : "off"}
          onClick={() => void commit({ autoImport: s.autoImport === false })}
        />
      </div>

      <div className="fc-set-group">
        <div className="fc-set-group-label">Workspaces</div>
        <Row
          label="Worktree directory"
          value={s.worktreeDirectory || DEFAULTS.worktreeDirectory}
          open={openRow === "worktreeDirectory"}
          onClick={() => toggle("worktreeDirectory")}
        >
          <InlineInput
            initial={s.worktreeDirectory ?? ""}
            placeholder={DEFAULTS.worktreeDirectory}
            onCancel={() => setOpenRow(null)}
            onSave={(v) => void commit({ worktreeDirectory: v.trim() || null })}
          />
        </Row>
        <Row
          label="tmux terminals"
          value={s.tmux ? "on" : "off"}
          onClick={() => void commit({ tmux: !s.tmux })}
        />
        <Row
          label="Preserve into new worktrees"
          shared={sharedTag("preservePatterns")}
          value={patterns.length > 0 ? patterns.join(" · ") : "—"}
          open={openRow === "preservePatterns"}
          onClick={() => toggle("preservePatterns")}
        >
          <InlineTextarea
            initial={patterns.join("\n")}
            placeholder={".env\n.env.local"}
            onCancel={() => setOpenRow(null)}
            onSave={(v) =>
              void commit({
                preservePatterns: v
                  .split(/\s+/)
                  .map((x) => x.trim())
                  .filter(Boolean),
              })
            }
          />
        </Row>
        <Row
          label="Shell setup"
          shared={sharedTag("shellSetup")}
          value={s.shellSetup || "—"}
          open={openRow === "shellSetup"}
          onClick={() => toggle("shellSetup")}
        >
          <InlineInput
            initial={s.shellSetup ?? ""}
            placeholder="e.g. source .envrc"
            onCancel={() => setOpenRow(null)}
            onSave={(v) => void commit({ shellSetup: v.trim() || null })}
          />
        </Row>
        <Row
          label="Task startup command"
          value={s.taskStartupCommand || "—"}
          open={openRow === "taskStartupCommand"}
          onClick={() => toggle("taskStartupCommand")}
        >
          <InlineInput
            initial={s.taskStartupCommand ?? ""}
            placeholder="e.g. omp — runs instead of a blank shell"
            onCancel={() => setOpenRow(null)}
            onSave={(v) => void commit({ taskStartupCommand: v.trim() || null })}
          />
        </Row>
        {byoiEnabled && (
        <Row
          label="Provision · terminate commands"
          value={
            s.workspaceProvider?.provisionCommand || s.workspaceProvider?.terminateCommand
              ? `${s.workspaceProvider?.provisionCommand || "—"} · ${
                  s.workspaceProvider?.terminateCommand || "—"
                }`
              : "—"
          }
          open={openRow === "workspaceProvider"}
          onClick={() => toggle("workspaceProvider")}
        >
          <div className="fc-set-editor fc-set-editor-pair">
            <input
              autoFocus
              defaultValue={s.workspaceProvider?.provisionCommand ?? ""}
              placeholder="provision command"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setOpenRow(null);
                }
              }}
              onBlur={(e) =>
                void commit({
                  workspaceProvider: {
                    type: "script",
                    provisionCommand: e.target.value.trim() || null,
                    terminateCommand: s.workspaceProvider?.terminateCommand,
                  },
                })
              }
            />
            <input
              defaultValue={s.workspaceProvider?.terminateCommand ?? ""}
              placeholder="terminate command"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setOpenRow(null);
                }
              }}
              onBlur={(e) =>
                void commit({
                  workspaceProvider: {
                    type: "script",
                    provisionCommand: s.workspaceProvider?.provisionCommand,
                    terminateCommand: e.target.value.trim() || null,
                  },
                })
              }
            />
            <div className="fc-set-editor-keys">
              script provider — saves on blur · esc closes
            </div>
          </div>
        </Row>
        )}
      </div>

      <div className="fc-set-group">
        <div className="fc-set-group-label">Agent</div>
        <Row
          label="Default agent · model"
          value={`${defaultAgentName(deps)} · default`}
          chevron
          open={openRow === "defaultAgent"}
          onClick={() => toggle("defaultAgent")}
        >
          <div className="fc-set-editor fc-set-menu">
            {deps
              .filter((d) => d.installed)
              .map((d) => (
                <button
                  key={d.providerId}
                  type="button"
                  className={`fc-set-menu-item${d.isDefault ? " current" : ""}`}
                  title={`Use ${d.name} for new sessions`}
                  onClick={() => void chooseAgent(d.providerId)}
                >
                  <span>{d.name}</span>
                  {d.isDefault && <span className="fc-tag-green">default</span>}
                </button>
              ))}
            {deps.filter((d) => d.installed).length === 0 && (
              <div className="fc-set-editor-keys">
                no installed agents — install one under App settings
              </div>
            )}
          </div>
        </Row>
        <Row
          label="Scripts"
          shared={sharedTag("scripts")}
          value={scriptsSummary}
          open={openRow === "scripts"}
          onClick={() => toggle("scripts")}
        >
          <div className="fc-set-editor fc-set-scripts">
            {(["setup", "run", "teardown"] as const).map((k) => (
              <label key={k} className="fc-set-script">
                <span className="fc-set-script-name">{k}</span>
                <textarea
                  rows={2}
                  defaultValue={s.scripts?.[k] ?? ""}
                  placeholder={`${k} script (shell lines)`}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      setOpenRow(null);
                    }
                  }}
                  onBlur={(e) => {
                    const v = e.target.value || null;
                    if (v !== (s.scripts?.[k] ?? null)) {
                      void commit({ scripts: { ...(s.scripts ?? {}), [k]: v } });
                    }
                  }}
                />
              </label>
            ))}
            <div className="fc-set-editor-keys">saves on blur · esc closes</div>
          </div>
        </Row>
        <Row
          label="Auto-run setup on new tasks"
          value={s.autoRunSetupScriptOnTaskCreation ? "on" : "off"}
          onClick={() =>
            void commit({
              autoRunSetupScriptOnTaskCreation: !s.autoRunSetupScriptOnTaskCreation,
            })
          }
        />
        <Row
          label="Auto-run run script on new tasks"
          value={s.autoRunRunScriptOnTaskCreation ? "on" : "off"}
          onClick={() =>
            void commit({
              autoRunRunScriptOnTaskCreation: !s.autoRunRunScriptOnTaskCreation,
            })
          }
        />
        {/* §8e "project settings carries the same switch in BOTH
            directions": the reversal half of the first-dispatch consent
            card. Unanswered (null) renders `off` because that is what the
            app actually does with it — the backend fails closed — and
            clicking from there writes an explicit `true`, so the card
            never asks about a project the user already decided here. */}
        <Row
          label="Feature dossiers"
          value={s.featureDossiers ? "on" : "off"}
          onClick={() => void commit({ featureDossiers: !s.featureDossiers })}
        />
        {/* #82 chain guard: both local, like every spend decision. Depth
            cap empty = built-in default (3 consecutive auto launches);
            budget empty = no budget. */}
        <Row
          label="Step chain depth cap"
          value={s.stepChainDepthCap != null ? String(s.stepChainDepthCap) : "default (3)"}
          open={openRow === "stepChainDepthCap"}
          onClick={() => toggle("stepChainDepthCap")}
        >
          <InlineInput
            initial={s.stepChainDepthCap != null ? String(s.stepChainDepthCap) : ""}
            placeholder="consecutive auto launches before the chain holds — empty = 3"
            onCancel={() => setOpenRow(null)}
            onSave={(v) => {
              const n = parseInt(v.trim(), 10);
              void commit({
                stepChainDepthCap: Number.isFinite(n) && n >= 0 ? n : null,
              });
            }}
          />
        </Row>
        <Row
          label="Step token budget"
          value={s.stepBudgetTokens != null ? s.stepBudgetTokens.toLocaleString() : "—"}
          open={openRow === "stepBudgetTokens"}
          onClick={() => toggle("stepBudgetTokens")}
        >
          <InlineInput
            initial={s.stepBudgetTokens != null ? String(s.stepBudgetTokens) : ""}
            placeholder="total reported tokens before chains hold — empty = no budget"
            onCancel={() => setOpenRow(null)}
            onSave={(v) => {
              const n = parseInt(v.trim(), 10);
              void commit({
                stepBudgetTokens: Number.isFinite(n) && n >= 0 ? n : null,
              });
            }}
          />
        </Row>
      </div>

      <div className="fc-set-group">
        <div className="fc-set-group-label">Archived</div>
        {archived.length === 0 && <Row label="No archived tasks" value="—" />}
        {archived.map((t) => (
          <div key={t.id} className="fc-set-row-wrap">
            <div className="fc-set-row static">
              <span className="fc-set-label">{t.name}</span>
              <span className="fc-set-value">
                {t.archivedAt ? t.archivedAt.slice(0, 10) : ""}
                <button
                  type="button"
                  className="fc-set-archived-action"
                  title="Restore — the task returns to the board"
                  onClick={() =>
                    void restoreTask(t.id)
                      .then(loadArchived)
                      .catch((e) => setError(String(e)))
                  }
                >
                  restore
                </button>
                <button
                  type="button"
                  className="fc-set-archived-action"
                  title="Delete — opens the destructive confirm"
                  onClick={() =>
                    useUi.getState().setDeleteTaskTarget({ projectId, taskId: t.id })
                  }
                >
                  delete…
                </button>
              </span>
            </div>
          </div>
        ))}
      </div>

      <div className="fc-set-spacer" />
      {notice && <p className="fc-set-notice">{notice}</p>}
      <div className="fc-set-footer">
        <span className="fc-set-legend">
          <span className="fc-tag-green-text">shared</span> lives in .fartCode.json · plain
          rows are local
        </span>
        <span className="fc-set-footer-key">⇧⌘s share local values</span>
      </div>
    </div>
  );
}

/* -- standalone path (Modals.tsx projectSettingsOpen): the unified surface
      opened directly at this project's pane. ------------------------------ */

export default function ProjectSettings({
  projectId,
  onClose,
}: {
  projectId: string;
  projectName: string;
  onClose: () => void;
}) {
  return <SettingsModal onClose={onClose} initialSection={`project:${projectId}`} />;
}
