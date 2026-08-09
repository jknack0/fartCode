// Keybinding dispatch hook (E14-01): registers all commands once, loads
// persisted user overrides from `view-state:app:keybindings`, and runs the
// single window keydown listener that computes scope context and dispatches.

import { useEffect } from "react";
import { getViewState, setViewState } from "./tauri";
import { registerAllCommands, registry } from "./commands";
import {
  ActiveBinding,
  CommandId,
  KEYBINDINGS_VIEW_STATE_KEY,
  applyUserOverrides,
  dispatchKey,
  hintFor,
  listBindings,
  resetToDefaults,
  ScopeContext,
} from "./registry";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  // xterm's hidden helper textarea is a key sink, not a text editor —
  // app chords (⌘W, ⌘⇧T, ...) must still fire while a terminal is focused.
  if (target.classList.contains("xterm-helper-textarea")) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

function scopeContext(e: KeyboardEvent): ScopeContext {
  const ui = useUi.getState();
  const sb = useSidebar.getState();
  return {
    projectView: sb.selectedProjectId !== null,
    taskView: sb.selectedTaskId !== null,
    editorFocused: isEditableTarget(e.target),
    modalOpen: ui.modalOpen(),
  };
}

let registered = false;

/** Installs the single keydown listener. Idempotent across HMR/re-mounts. */
export function useCommands(): void {
  useEffect(() => {
    if (!registered) {
      registerAllCommands();
      registered = true;
      // Restore persisted user overrides (user wins over defaults).
      void getViewState(KEYBINDINGS_VIEW_STATE_KEY)
        .then((saved) => {
          if (saved && typeof saved === "object") {
            applyUserOverrides(registry, saved as Record<string, string[]>);
            useUi.getState().bumpBindings();
          }
        })
        .catch(() => {});
    }
    const onKey = (e: KeyboardEvent) => {
      dispatchKey(registry, e, scopeContext(e));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Live binding list for the settings UI + hint rendering (re-reads the
 * registry on every call; the settings modal re-renders on demand). */
export function bindings(): ActiveBinding[] {
  return listBindings(registry);
}

/** Hint for one command ("" when unbound). */
export function hint(id: Parameters<typeof hintFor>[1]): string {
  return hintFor(registry, id);
}

/** Runs a registered command by id (clickable hint chips, palette rows). */
export function runCommand(id: CommandId): void {
  registry.commands.find((c) => c.id === id)?.run();
}

/** Persists an override for one command ({commandId -> [chord strings]});
 * pass null to clear that command back to defaults. Re-applies all saved
 * overrides on top of the defaults so the registry stays consistent. */
export async function saveOverride(
  commandId: string,
  chords: string[] | null,
): Promise<void> {
  const saved =
    ((await getViewState(KEYBINDINGS_VIEW_STATE_KEY)) as Record<string, string[]> | null) ?? {};
  if (chords === null) {
    delete saved[commandId];
  } else {
    saved[commandId] = chords;
  }
  await setViewState(KEYBINDINGS_VIEW_STATE_KEY, saved);
  resetToDefaults(registry);
  applyUserOverrides(registry, saved);
  useUi.getState().bumpBindings();
}

/** Clears every custom binding (acceptance: restores defaults). */
export async function clearAllOverrides(): Promise<void> {
  await setViewState(KEYBINDINGS_VIEW_STATE_KEY, {});
  resetToDefaults(registry);
  useUi.getState().bumpBindings();
}
