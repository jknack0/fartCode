// Project Settings panel (E1-05): per-project configuration with defaults
// shown for unset fields, worktree-directory validation, and share-with-team.
import { useEffect, useState } from "react";
import {
  DefaultBranchDto,
  ProjectSettingsDto,
  getProjectSettings,
  shareWithTeam,
  updateProjectSettings,
} from "../lib/tauri";

const DEFAULTS = {
  worktreeDirectory: "~/ade/worktrees",
  defaultBranch: "main",
  baseRemote: "origin",
};

export default function ProjectSettings({
  projectId,
  projectName,
  onClose,
}: {
  projectId: string;
  projectName: string;
  onClose: () => void;
}) {
  const [s, setS] = useState<ProjectSettingsDto>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    getProjectSettings(projectId)
      .then(setS)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [projectId]);

  const set = (patch: Partial<ProjectSettingsDto>) => {
    setS((prev) => ({ ...prev, ...patch }));
    setDirty(true);
    setNotice(null);
  };

  const defaultBranchName = (b?: DefaultBranchDto | null): string =>
    typeof b === "string" ? b : b?.name ?? "";

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const saved = await updateProjectSettings(projectId, s);
      setS(saved);
      setDirty(false);
      setNotice("Saved");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const share = async () => {
    setError(null);
    try {
      const shared = await shareWithTeam(projectId);
      const fresh = await getProjectSettings(projectId);
      setS(fresh);
      setDirty(false);
      setNotice(
        shared
          ? "Shared to .ade.json — commit it to give teammates these defaults"
          : "Nothing local to share — all shareable fields already live in .ade.json",
      );
    } catch (e) {
      setError(String(e));
    }
  };

  if (loading) return <div className="modal-backdrop"><div className="modal">Loading…</div></div>;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal settings-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Project settings — {projectName}</h2>
        {error && <p className="error">{error}</p>}
        {notice && <p className="notice">{notice}</p>}

        <h3>Workspace</h3>
        <label>
          Worktree directory
          <input
            value={s.worktreeDirectory ?? ""}
            placeholder={DEFAULTS.worktreeDirectory}
            onChange={(e) => set({ worktreeDirectory: e.target.value || null })}
          />
        </label>
        <p className="hint">
          Absolute path or <code>~</code>-relative. Relative paths are rejected.
        </p>

        <label>
          Default branch
          <input
            value={defaultBranchName(s.defaultBranch)}
            placeholder={DEFAULTS.defaultBranch}
            onChange={(e) => {
              const value = e.target.value || null;
              // Preserve remote-tracking when the loaded branch is {name, remote: true}.
              if (value && typeof s.defaultBranch === "object" && s.defaultBranch !== null && s.defaultBranch.remote) {
                set({ defaultBranch: { name: value, remote: true } });
              } else {
                set({ defaultBranch: value });
              }
            }}
          />
        </label>

        <div className="row">
          <label>
            Base remote
            <input
              value={s.baseRemote ?? ""}
              placeholder={DEFAULTS.baseRemote}
              onChange={(e) => set({ baseRemote: e.target.value || null })}
            />
          </label>
          <label>
            Push remote
            <input
              value={s.pushRemote ?? ""}
              placeholder="same as base"
              onChange={(e) => set({ pushRemote: e.target.value || null })}
            />
          </label>
        </div>

        <h3>Behavior</h3>
        <label className="check">
          <input
            type="checkbox"
            checked={s.tmux ?? false}
            onChange={(e) => set({ tmux: e.target.checked })}
          />
          Use tmux for terminals
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={s.autoRunSetupScriptOnTaskCreation ?? false}
            onChange={(e) => set({ autoRunSetupScriptOnTaskCreation: e.target.checked })}
          />
          Auto-run setup script on task creation
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={s.autoRunRunScriptOnTaskCreation ?? false}
            onChange={(e) => set({ autoRunRunScriptOnTaskCreation: e.target.checked })}
          />
          Auto-run run script on task creation
        </label>

        <h3>Lifecycle scripts</h3>
        {(["setup", "run", "teardown"] as const).map((k) => (
          <label key={k}>
            {k}
            <textarea
              rows={2}
              value={s.scripts?.[k] ?? ""}
              placeholder={`${k} script (shell lines)`}
              onChange={(e) =>
                set({
                  scripts: {
                    ...(s.scripts ?? {}),
                    [k]: e.target.value || null,
                  },
                })
              }
            />
          </label>
        ))}
        <label>
          Shell setup (prepended to every script)
          <input
            value={s.shellSetup ?? ""}
            placeholder="e.g. source .envrc"
            onChange={(e) => set({ shellSetup: e.target.value || null })}
          />
        </label>

        <label>
          Preserve patterns (one per line)
          <textarea
            rows={3}
            value={(s.preservePatterns ?? []).join("\n")}
            placeholder={".env\n.env.local"}
            onChange={(e) =>
              set({
                preservePatterns: e.target.value
                  .split(/\s+/)
                  .map((x) => x.trim())
                  .filter(Boolean),
              })
            }
          />
        </label>

        <h3>Workspace provider</h3>
        <p className="hint">
          Phase 0 supports the <code>script</code> provider (commands run in
          the worktree); ssh/byoi arrive with their tickets.
        </p>
        <label>
          Provision command
          <input
            value={s.workspaceProvider?.provisionCommand ?? ""}
            onChange={(e) =>
              set({
                workspaceProvider: {
                  type: "script",
                  provisionCommand: e.target.value || null,
                  terminateCommand: s.workspaceProvider?.terminateCommand,
                },
              })
            }
          />
        </label>
        <label>
          Terminate command
          <input
            value={s.workspaceProvider?.terminateCommand ?? ""}
            onChange={(e) =>
              set({
                workspaceProvider: {
                  type: "script",
                  provisionCommand: s.workspaceProvider?.provisionCommand,
                  terminateCommand: e.target.value || null,
                },
              })
            }
          />
        </label>

        <div className="modal-actions">
          <button onClick={share} disabled={saving || dirty}>
            Share with team
          </button>
          <button onClick={onClose}>Close</button>
          <button className="primary" onClick={save} disabled={saving || !dirty}>
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
