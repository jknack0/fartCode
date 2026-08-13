// Lifecycle script runs (design_handoff_v2 5a/7b, E1-06): per task, per
// script type, the terminal id + live state of the LAST run. The drawer
// (⌘J) and the header launchers read this store; lifecycle terminals no
// longer appear as pane tabs (7b — the drawer owns them).
//
// It also tracks the task's LIVE agent terminal (5a/5b): the header dot
// and the empty state's "stopped · elapsed" derive from the agent, never
// from task.status — hydrate() snapshots `kind === "agent" && running`,
// noteAgentSpawn() covers agents this frontend just opened, and the exit
// event flips `running` off and stamps `exitedAt`.
//
// Nothing is persisted here: the backend retains lifecycle terminal
// entries (with exitCode/running) after the script exits, so hydrate()
// rebuilds the state from terminal_list_for_task and the exit event keeps
// it live.
import { create } from "zustand";
import {
  onTerminalExited,
  terminalListForTask,
  terminalOpenLifecycle,
} from "../lib/tauri";
import { wireEvents } from "../lib/wireEvents";

export type ScriptType = "setup" | "run" | "teardown";
export const SCRIPT_TYPES: readonly ScriptType[] = ["setup", "run", "teardown"];

export function isScriptType(s: string | null | undefined): s is ScriptType {
  return s === "setup" || s === "run" || s === "teardown";
}

/** State of a script's last run. `terminalId === null` = never run. */
export interface ScriptRun {
  terminalId: string | null;
  running: boolean;
  exitCode: number | null;
}

export type TaskScripts = Record<ScriptType, ScriptRun>;

/** The task's live agent terminal(s) — ADR-0033 says one, but ids is a
 * list so a stray second agent (omp next to claude) still reads as
 * "running". `exitedAt` is the wall-clock ms of the last agent exit seen
 * this session; null = never observed (fall back to statusChangedAt). */
export interface AgentState {
  ids: string[];
  running: boolean;
  exitedAt: number | null;
}

const neverRun = (): ScriptRun => ({
  terminalId: null,
  running: false,
  exitCode: null,
});

const emptyTask = (): TaskScripts => ({
  setup: neverRun(),
  run: neverRun(),
  teardown: neverRun(),
});

interface ScriptsState {
  byTask: Record<string, TaskScripts>;
  agentByTask: Record<string, AgentState>;

  /** Rebuilds a task's script + agent state from the backend's terminal
   * list (lifecycle entries carry exitCode/running). Called by the task
   * view on mount; safe to repeat. */
  hydrate: (taskId: string) => Promise<void>;
  /** Starts (or reattaches — backend dedupes an in-flight run of the same
   * type) the script and re-points the drawer at the fresh terminal via
   * the updated `terminalId`. Returns the terminal id. */
  rerun: (taskId: string, scriptType: ScriptType) => Promise<string>;
  /** Records an agent terminal this frontend just opened, so the header
   * dot flips to running without waiting for a re-hydrate. */
  noteAgentSpawn: (taskId: string, terminalId: string) => void;
}

// One process-wide exit subscription, wired lazily on first hydrate so no
// app-shell wiring is needed (App.tsx stays untouched).
let exitWired = false;

// Exit events outrank terminal-list snapshots: a hydrate whose list was
// fetched before an exit landed must not resurrect the dead id (stuck
// amber dot). Session-scoped; a session sees at most dozens of terminals.
const exitedTerminals = new Set<string>();

function wireExitEvents(): void {
  if (exitWired) return;
  exitWired = true;
  wireEvents(onTerminalExited, ({ terminalId, exitCode }) => {
    exitedTerminals.add(terminalId);
    let setupSucceededTask: string | null = null;
    useScripts.setState((s) => {
      for (const [taskId, scripts] of Object.entries(s.byTask)) {
        for (const k of SCRIPT_TYPES) {
          if (scripts[k].terminalId !== terminalId) continue;
          if (k === "setup" && exitCode === 0) setupSucceededTask = taskId;
          return {
            byTask: {
              ...s.byTask,
              [taskId]: {
                ...scripts,
                [k]: { terminalId, running: false, exitCode },
              },
            },
          };
        }
      }
      // Agent terminals (5a dot / 5b "stopped · elapsed"): drop the id and
      // stamp the exit time.
      for (const [taskId, agent] of Object.entries(s.agentByTask)) {
        if (!agent.ids.includes(terminalId)) continue;
        const ids = agent.ids.filter((id) => id !== terminalId);
        return {
          agentByTask: {
            ...s.agentByTask,
            [taskId]: { ids, running: ids.length > 0, exitedAt: Date.now() },
          },
        };
      }
      return s;
    });
    // A green setup may auto-launch the agent (backend gate) — refresh the
    // snapshot so the header dot picks the new terminal up.
    if (setupSucceededTask) void useScripts.getState().hydrate(setupSucceededTask);
  });
}

export const useScripts = create<ScriptsState>((set) => ({
  byTask: {},
  agentByTask: {},

  hydrate: async (taskId) => {
    wireExitEvents();
    const terms = await terminalListForTask(taskId).catch(() => []);
    const next = emptyTask();
    const liveAgentIds: string[] = [];
    for (const t of terms) {
      if (t.kind === "agent" && t.running) liveAgentIds.push(t.id);
      if (t.kind !== "lifecycle" || !isScriptType(t.scriptType)) continue;
      next[t.scriptType] = {
        terminalId: t.id,
        running: t.running,
        exitCode: t.exitCode,
      };
    }
    set((s) => {
      // A rerun() that landed while this list was in flight knows more
      // than the snapshot — never clobber a live run with "never ran".
      const prev = s.byTask[taskId];
      if (prev) {
        for (const k of SCRIPT_TYPES) {
          if (prev[k].running && next[k].terminalId === null) next[k] = prev[k];
        }
      }
      // Same guard for agents: a spawn noted while the list was in flight
      // stays live (the exit event is what removes ids) — but nothing the
      // exit event already buried comes back from a stale snapshot.
      const prevAgent = s.agentByTask[taskId];
      const ids = liveAgentIds.filter((id) => !exitedTerminals.has(id));
      for (const id of prevAgent?.ids ?? []) {
        if (!ids.includes(id) && !exitedTerminals.has(id)) ids.push(id);
      }
      return {
        byTask: { ...s.byTask, [taskId]: next },
        agentByTask: {
          ...s.agentByTask,
          [taskId]: {
            ids,
            running: ids.length > 0,
            exitedAt: prevAgent?.exitedAt ?? null,
          },
        },
      };
    });
  },

  rerun: async (taskId, scriptType) => {
    wireExitEvents();
    const terminalId = await terminalOpenLifecycle(taskId, scriptType, 24, 80);
    set((s) => ({
      byTask: {
        ...s.byTask,
        [taskId]: {
          ...(s.byTask[taskId] ?? emptyTask()),
          [scriptType]: { terminalId, running: true, exitCode: null },
        },
      },
    }));
    return terminalId;
  },

  noteAgentSpawn: (taskId, terminalId) => {
    wireExitEvents();
    set((s) => {
      const prev = s.agentByTask[taskId];
      if (prev?.ids.includes(terminalId)) return s;
      const ids = [...(prev?.ids ?? []), terminalId];
      return {
        agentByTask: {
          ...s.agentByTask,
          [taskId]: { ids, running: true, exitedAt: prev?.exitedAt ?? null },
        },
      };
    });
  },
}));
