// Unified settings surface (design_handoff_v2 7c): 170px left nav —
// App / Keys / one row per project — with the active row pulled into the
// gutter by the accent bar. App pane = agents on this machine (7d) +
// provider accounts; Keys pane = the E14-01 shortcut editor restyled to
// rows; project panes = ProjectSettingsPane (full field map). Rendering is
// gated by Modals.tsx (settingsOpen / projectSettingsOpen) — the default
// export here draws the surface unconditionally.
import { Fragment, useReducer, useState } from "react";
import { chordFromEvent, formatChord, isBindableChord } from "../lib/keychord";
import { bindings, clearAllOverrides, hint, saveOverride } from "../lib/useCommands";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";
import AgentsList from "./AgentsList";
import { ColumnsPane } from "./ColumnsEditor";
import { ProjectSettingsPane } from "./ProjectSettings";
import ProviderAccounts from "./ProviderAccounts";

const SCOPE_LABELS: Record<string, string> = {
  global: "Everywhere",
  "app-view": "Everywhere",
  "project-view": "Project open",
  "task-view": "Task open",
  editor: "Editor focused",
  modal: "Dialogs",
};

const SCOPE_ORDER = ["Everywhere", "Project open", "Task open", "Editor focused", "Dialogs"];

/* -- Keys pane: the shortcut editor as rows ------------------------------ */

function KeysPane() {
  // Registry mutations are outside zustand — bump to re-render after them.
  const [, bump] = useReducer((n: number) => n + 1, 0);
  const [editing, setEditing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const list = bindings();

  const captureChord = (commandId: string) => (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setEditing(null);
      setError(null);
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    // Ignore bare modifier presses — wait for the real key of the chord.
    if (["Meta", "Alt", "Shift", "Control"].includes(e.key)) return;
    const chord = chordFromEvent(e);
    if (!isBindableChord(chord)) {
      setError("Use a modifier chord (⌘/Ctrl/⌥) or Tab/Escape/F-keys.");
      return;
    }
    const target = list.find((b) => b.id === commandId);
    if (!target) return;
    const conflict = list.find(
      (b) =>
        b.id !== commandId &&
        b.scope === target.scope &&
        b.chords.some(
          (c) =>
            c.meta === chord.meta &&
            c.ctrl === chord.ctrl &&
            c.alt === chord.alt &&
            c.shift === chord.shift &&
            c.key === chord.key,
        ),
    );
    if (conflict) {
      setError(`${formatChord(chord)} is already bound to “${conflict.label}”.`);
      return;
    }
    void saveOverride(commandId, [formatChord(chord)]).then(() => {
      setEditing(null);
      setError(null);
      bump();
    });
  };

  const groups = SCOPE_ORDER.map((label) => ({
    label,
    rows: list.filter((b) => (SCOPE_LABELS[b.scope] ?? b.scope) === label),
  })).filter((g) => g.rows.length > 0);

  return (
    <>
      {error && <p className="fc-set-error">{error}</p>}
      {groups.map((g) => (
        <div className="fc-set-group" key={g.label}>
          <div className="fc-set-group-label">{g.label}</div>
          {g.rows.map((b) => (
            <div className="fc-key-row" key={b.id}>
              <span className="fc-key-label">
                {b.label}
                {b.customized && (
                  <span className="fc-key-custom" title="Customized">
                    custom
                  </span>
                )}
              </span>
              <span className="fc-key-right">
                {editing === b.id ? (
                  <span
                    className="fc-key-editing"
                    tabIndex={0}
                    onKeyDown={captureChord(b.id)}
                    ref={(el) => el?.focus()}
                  >
                    press a chord… esc cancels
                  </span>
                ) : (
                  <button
                    className={`fc-key-chord${b.customized ? " customized" : ""}`}
                    title={
                      b.hint
                        ? `Remap ${b.label} — click, then press the new chord`
                        : `Bind ${b.label}`
                    }
                    onClick={() => {
                      setError(null);
                      setEditing(b.id);
                    }}
                  >
                    {b.hint || "unbound"}
                  </button>
                )}
                {b.customized && editing !== b.id && (
                  <button
                    className="fc-key-reset"
                    title="Reset to default"
                    onClick={() => void saveOverride(b.id, null).then(bump)}
                  >
                    ↺
                  </button>
                )}
              </span>
            </div>
          ))}
        </div>
      ))}
      <div className="fc-set-spacer" />
      <div className="fc-set-footer">
        <span className="fc-set-legend">click a binding · press the new chord</span>
        <button
          className="fc-set-footer-action"
          title="Remove every custom binding"
          onClick={() => void clearAllOverrides().then(bump)}
        >
          clear custom bindings
        </button>
      </div>
    </>
  );
}

/* -- shell --------------------------------------------------------------- */

export default function SettingsModal({
  onClose,
  initialSection = "app",
}: {
  onClose: () => void;
  initialSection?: string;
}) {
  const [section, setSection] = useState(initialSection);
  const projects = useSidebar((s) => s.projects);
  // Hints re-render live when bindings change (spec: subscribe bindingsVersion).
  useUi((s) => s.bindingsVersion);

  // Sections: "app" · "keys" · "project:<id>" · "project:<id>:columns".
  const columnsChild = section.startsWith("project:") && section.endsWith(":columns");
  const sectionProjectId = section.startsWith("project:")
    ? section.slice("project:".length, columnsChild ? -":columns".length : undefined)
    : null;
  const activeProject = sectionProjectId
    ? projects.find((p) => p.id === sectionProjectId) ?? null
    : null;
  const title =
    section === "app"
      ? "App"
      : section === "keys"
        ? "Keys"
        : columnsChild
          ? `Columns · ${activeProject?.name ?? ""}`
          : activeProject?.name ?? "";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="fc-settings" onClick={(e) => e.stopPropagation()}>
        <nav className="fc-set-nav">
          <button
            className={`fc-set-nav-row${section === "app" ? " active" : ""}`}
            onClick={() => setSection("app")}
          >
            App
          </button>
          <button
            className={`fc-set-nav-row${section === "keys" ? " active" : ""}`}
            onClick={() => setSection("keys")}
          >
            Keys
          </button>
          {projects.map((p) => (
            <Fragment key={p.id}>
              <button
                className={`fc-set-nav-row${section === `project:${p.id}` ? " active" : ""}`}
                onClick={() => setSection(`project:${p.id}`)}
              >
                {p.name}
              </button>
              <button
                className={`fc-set-nav-row child${
                  section === `project:${p.id}:columns` ? " active" : ""
                }`}
                onClick={() => setSection(`project:${p.id}:columns`)}
              >
                Columns
              </button>
            </Fragment>
          ))}
        </nav>
        <div className="fc-set-pane">
          <div className="fc-set-pane-head">
            <span className="fc-set-pane-title">{title}</span>
            <button className="fc-set-esc" onClick={onClose} title="Close settings">
              {(hint("close-modal") || "esc").toLowerCase()}
            </button>
          </div>
          {section === "app" && (
            <div className="fc-set-pane-body">
              <AgentsList />
              <ProviderAccounts />
              <div className="fc-set-spacer" />
            </div>
          )}
          {section === "keys" && (
            <div className="fc-set-pane-body">
              <KeysPane />
            </div>
          )}
          {activeProject &&
            (columnsChild ? (
              <ColumnsPane projectId={activeProject.id} />
            ) : (
              <ProjectSettingsPane projectId={activeProject.id} />
            ))}
        </div>
      </div>
    </div>
  );
}
