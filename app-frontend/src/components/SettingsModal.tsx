// App settings modal (E14-01): shortcut customization — every registered
// command with its live binding, remap by pressing the new chord, reset per
// command or clear all (restores the default map and removes hints).
import { useReducer, useState } from "react";
import { chordFromEvent, formatChord, isBindableChord } from "../lib/keychord";
import { bindings, clearAllOverrides, saveOverride } from "../lib/useCommands";
import { useUi } from "../store/ui";

export default function SettingsModal({ onClose }: { onClose: () => void }) {
  // Registry mutations are outside zustand — bump to re-render after them.
  const [, bump] = useReducer((n: number) => n + 1, 0);
  const [editing, setEditing] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const list = bindings();
  const settingsOpen = useUi((s) => s.settingsOpen);

  if (!settingsOpen) return null;

  const captureChord = (commandId: string) => (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
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

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal settings-modal shortcuts-settings" onClick={(e) => e.stopPropagation()}>
        <h2>Settings</h2>

        <h3>Keyboard shortcuts</h3>
        <p className="hint">
          Click a binding and press the new chord. ⌘ maps to Ctrl on
          Windows/Linux. Changes apply immediately and survive restarts.
        </p>
        {error && <p className="error">{error}</p>}

        <table className="shortcut-table">
          <thead>
            <tr>
              <th>Command</th>
              <th>Scope</th>
              <th>Binding</th>
            </tr>
          </thead>
          <tbody>
            {list.map((b) => (
              <tr key={b.id}>
                <td className="shortcut-label">
                  {b.label}
                  {b.customized && <span className="customized-mark" title="Customized" />}
                </td>
                <td className="shortcut-scope">{b.scope}</td>
                <td>
                  {editing === b.id ? (
                    <span
                      className="shortcut-editing"
                      tabIndex={0}
                      onKeyDown={captureChord(b.id)}
                      ref={(el) => el?.focus()}
                    >
                      press a chord… (Esc cancels)
                    </span>
                  ) : (
                    <button
                      className={`shortcut-chord${b.customized ? " customized" : ""}`}
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
                      className="shortcut-reset"
                      title="Reset to default"
                      onClick={() => void saveOverride(b.id, null).then(bump)}
                    >
                      ↺
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>

        <div className="modal-actions">
          <button
            title="Remove every custom binding"
            onClick={() => void clearAllOverrides().then(bump)}
          >
            Clear all custom bindings
          </button>
          <button className="primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
