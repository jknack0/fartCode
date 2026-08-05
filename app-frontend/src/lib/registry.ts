// Keybinding registry + default map + scoping engine (E14-01).
//
// Every keyboard shortcut in the app is a registered Command. Commands are
// dispatched by a single window listener against scopes in precedence order:
// modal > editor > task-view > project-view > global.
// Higher-priority scopes consume the chord first; a match prevents
// fall-through.
//
// E14-02 owns the settings UI + hint rendering; this module owns the
// registry, the default keymap, conflict detection, and persistence of
// user overrides (user wins over defaults).

import {
  KeyChord,
  chordFromEvent,
  chordsEqual,
  formatChord,
  isBindableChord,
  parseChord,
} from "./keychord";

export type Scope =
  | "global"
  | "app-view"
  | "project-view"
  | "task-view"
  | "editor"
  | "modal";

/** Precedence: modal first, global last. */
const SCOPE_PRECEDENCE: Scope[] = [
  "modal",
  "editor",
  "task-view",
  "project-view",
  "app-view",
  "global",
];

export type CommandId =
  | "open-command-palette"
  | "open-settings"
  | "new-project"
  | "add-task"
  | "delete-task"
  | "new-terminal"
  | "new-terminal-right-split"
  | "open-omp"
  | "toggle-sidebar"
  | "toggle-right-panel"
  | "previous-task"
  | "next-task"
  | "close-tab"
  | "split-pane"
  | "next-tab"
  | "previous-tab"
  | "close-modal"
  | "jump-to-tab-1"
  | "jump-to-tab-2"
  | "jump-to-tab-3"
  | "jump-to-tab-4"
  | "jump-to-tab-5"
  | "jump-to-tab-6"
  | "jump-to-tab-7"
  | "jump-to-tab-8"
  | "jump-to-tab-9";

export interface CommandDef {
  id: CommandId;
  label: string;
  /** Human-readable chords (default keymap); the first is the displayed hint. */
  defaultKeys: string[];
  scope: Scope;
  /** When true the command is skipped while an editor/input is focused —
   * the editor consumes it (⌘1–9, ⌘W, ⌘⌥↑/↓, ⌘\, ⌘Backspace). Default:
   * fires everywhere (⌘K, ⌘,, Ctrl+Tab, ⌘Enter …). */
  skipInEditor?: boolean;
  run: () => void;
}

export interface ActiveBinding {
  id: CommandId;
  label: string;
  scope: Scope;
  chords: KeyChord[];
  hint: string;
  customized: boolean;
  run: () => void;
}

export interface ScopeContext {
  projectView: boolean;
  taskView: boolean;
  editorFocused: boolean;
  modalOpen: boolean;
}

interface Registry {
  commands: CommandDef[];
  bindings: Record<string, KeyChord[]>;
}

export const KEYBINDINGS_VIEW_STATE_KEY = "view-state:app:keybindings";

function parseDefaults(def: CommandDef): KeyChord[] {
  return def.defaultKeys
    .map((k) => parseChord(k))
    .filter((c): c is KeyChord => c !== null && isBindableChord(c));
}

export function createRegistry(): Registry {
  return { commands: [], bindings: {} };
}

/** Registers a command. A chord already bound to another command in the
 * same scope is rejected with a console warning (acceptance criterion). */
export function registerCommand(registry: Registry, def: CommandDef): void {
  if (registry.commands.some((c) => c.id === def.id)) {
    console.warn(`[keybindings] duplicate command id: ${def.id}`);
    return;
  }
  const chords: KeyChord[] = [];
  for (const chord of parseDefaults(def)) {
    const conflict = registry.commands.find(
      (cmd) =>
        cmd.scope === def.scope &&
        (registry.bindings[cmd.id] ?? []).some((c) => chordsEqual(c, chord)),
    );
    if (conflict) {
      console.warn(
        `[keybindings] ${formatChord(chord)} in scope '${def.scope}' already bound to '${conflict.id}' — rejecting for '${def.id}'`,
      );
      continue;
    }
    chords.push(chord);
  }
  registry.commands.push(def);
  registry.bindings[def.id] = chords;
}

/** Formatted primary binding for hint rendering (E14-02). */
export function hintFor(registry: Registry, id: CommandId): string {
  const chords = registry.bindings[id];
  if (!chords || chords.length === 0) return "";
  return formatChord(chords[0]);
}

/** All bindings with current chords + customization flag, registration order. */
export function listBindings(registry: Registry): ActiveBinding[] {
  return registry.commands.map((cmd) => {
    const chords = registry.bindings[cmd.id] ?? [];
    const defaults = parseDefaults(cmd);
    const customized =
      chords.length !== defaults.length || chords.some((c, i) => !chordsEqual(c, defaults[i]));
    return {
      id: cmd.id,
      label: cmd.label,
      scope: cmd.scope,
      chords,
      hint: chords.length > 0 ? formatChord(chords[0]) : "",
      customized,
      run: cmd.run,
    };
  });
}

/** Applies user overrides ({commandId -> chord strings}); unknown ids,
 * unparseable, and conflicting chords are ignored (with a warning). */
export function applyUserOverrides(
  registry: Registry,
  overrides: Record<string, string[]>,
): void {
  for (const cmd of registry.commands) {
    const raw = overrides[cmd.id];
    if (!raw || !Array.isArray(raw)) continue;
    const chords = raw
      .map((k) => parseChord(k))
      .filter((c): c is KeyChord => c !== null && isBindableChord(c));
    if (chords.length === 0) continue;
    const conflict = chords.find((chord) =>
      registry.commands.some(
        (other) =>
          other.id !== cmd.id &&
          other.scope === cmd.scope &&
          (registry.bindings[other.id] ?? []).some((c) => chordsEqual(c, chord)),
      ),
    );
    if (conflict) {
      console.warn(
        `[keybindings] override ${formatChord(conflict)} for '${cmd.id}' conflicts in scope '${cmd.scope}' — ignored`,
      );
      continue;
    }
    registry.bindings[cmd.id] = chords;
  }
}

/** Resets every command to its default chords. */
export function resetToDefaults(registry: Registry): void {
  for (const cmd of registry.commands) {
    registry.bindings[cmd.id] = parseDefaults(cmd);
  }
}

function activeScopes(ctx: ScopeContext): Set<Scope> {
  const scopes = new Set<Scope>(["global", "app-view"]);
  // View scopes are suspended while a modal is open — modal keys (Esc) own
  // the keyboard; task commands must not fire underneath a dialog
  // (matches E2-10's modal guard).
  if (!ctx.modalOpen) {
    if (ctx.projectView) scopes.add("project-view");
    if (ctx.taskView) scopes.add("task-view");
  }
  if (ctx.editorFocused) scopes.add("editor");
  if (ctx.modalOpen) scopes.add("modal");
  return scopes;
}

/** Dispatches a key event against the registry. Returns the command id that
 * ran, or null. Scope precedence: modal > editor > task-view >
 * project-view > global; within a scope the first matching binding wins. */
export function dispatchKey(
  registry: Registry,
  e: KeyboardEvent,
  ctx: ScopeContext,
): CommandId | null {
  if (e.repeat) return null;
  const chord = chordFromEvent(e);
  const scopes = activeScopes(ctx);

  for (const scope of SCOPE_PRECEDENCE) {
    if (!scopes.has(scope)) continue;
    for (const cmd of registry.commands) {
      if (cmd.scope !== scope) continue;
      // Editor exemption: skipInEditor commands are consumed by the focused
      // editor; modal scope bypasses this (modal inputs must not swallow
      // modal keys).
      if (ctx.editorFocused && scope !== "modal" && cmd.skipInEditor) continue;
      if ((registry.bindings[cmd.id] ?? []).some((c) => chordsEqual(c, chord))) {
        e.preventDefault();
        cmd.run();
        return cmd.id;
      }
    }
  }
  return null;
}
