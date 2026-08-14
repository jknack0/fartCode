// Step engine state + the launch directive (E18-04/05, ADR-0037).
//
// This is APP-LIFETIME on purpose. `step:launch` is a directive — the
// backend provisions the task and emits it, and the frontend is what
// actually opens the agent terminal (fartcode-core/src/events.rs). Acting
// on that directive from inside BoardView was the bug: opening a session
// navigates to the task view, which unmounts BoardView, which tears the
// listener down — so a settle-CHAINED launch (step A advances into step B
// while you are watching step A) arrived with nobody listening and the
// pipeline stalled silently. The same unmount wiped the step-done and
// queued flags, so returning to the board showed a bare card.
//
// So: one subscription wired from App.tsx for the process lifetime, and
// the flags live here rather than in component state. Remounts and project
// switches no longer lose anything.
//
// Delivery is EVENT-ONLY. Every launch emits `step:launch`
// (fartcode-app/src/step_engine.rs `launch_step`), command-path and
// settle-chained alike, so the event is a complete channel on its own.
// The command's `EnterOutcomeDto.launch` is therefore deliberately NOT
// acted on — one path means no dedupe window, and no dedupe window means a
// genuine second dispatch can never be swallowed.

import { create } from "zustand";
import {
  onFartcodeEvent,
  stepParkedList,
  terminalOpenAgent,
  terminalWrite,
  waitForTerminalReady,
  type FartcodeEvent,
  type ParkReason,
} from "../lib/tauri";
import { useScripts } from "./scripts";
import { useSidebar } from "./sidebar";

/** Per-issue engine state. Nothing is stored backend-side (step-done is
 * derived state by doctrine), so this is the session's memory of it. */
export interface StepFlags {
  /** step:settled landed for this column and the column holds. */
  settledColumnId?: string;
  /** step:queued parked a step here. `confirm` awaits the human;
   * `agent_busy` awaits the task's live agent exiting and then fires
   * itself — there is nothing to confirm. */
  queuedColumnId?: string;
  /** Which gate holds it. */
  queuedReason?: ParkReason;
  /** step:chain_held — the guard refused the next automatic launch while
   * the card sat here (#82). Cleared by the next launch/entry. */
  heldColumnId?: string;
  /** Why: "depth" | "cycle" | "budget". */
  holdReason?: string;
  /** Agent the parked step will run, for the confirm copy. */
  queuedProvider?: string;
  queuedModel?: string | null;
  queuedEffort?: string | null;
}

/** A dispatch in flight, from the drag that began it until the prompt is
 * pasted (or the directive is refused). Presence is the whole signal — no
 * phase bookkeeping. `taskId` arrives with `step:launch`, which is what
 * lets the flyout count the task as Running before its terminal is open. */
export interface LaunchState {
  taskId?: string;
}

interface StepsState {
  byIssue: Record<string, StepFlags>;
  /** Projects whose parks were re-seeded from `step_parked_list` (E18-09
   * rehydration) — once per project per webview lifetime. */
  hydrated: Record<string, boolean>;
  /** Surfaced by the board — a directive that could not be carried out
   * (agent binary missing, PTY refused) must not fail silently. */
  error: string | null;
  setError: (message: string | null) => void;
  /** A park the UI raised from a command outcome rather than the event —
   * see `notePark`. */
  notePark: (
    issueId: string,
    columnId: string,
    agent?: {
      provider?: string;
      model?: string | null;
      effort?: string | null;
      reason?: ParkReason;
    },
  ) => void;
  /** Drops the queued flag once the confirm fires or is dismissed. */
  clearPark: (issueId: string) => void;
  /** Drops everything known about an issue (deletion, or a fresh launch). */
  clearIssue: (issueId: string) => void;
  /** A dispatch in flight (drag → prompt pasted, or refused). Presence is
   * the whole signal — no phase bookkeeping. `taskId` lands with
   * `step:launch`, which lets the flyout count the task as Running before
   * its terminal is open. */
  launchingByIssue: Record<string, LaunchState>;
  /** Mark a dispatch begun from a user gesture (drag / confirm / advance). */
  beginDispatch: (issueId: string) => void;
  /** Attach the provisioned task id once `step:launch` arrives. */
  noteLaunch: (issueId: string, taskId: string) => void;
  /** Clear the dispatch (prompt pasted, refused, or errored). */
  endLaunch: (issueId: string) => void;
}

/** Clears observed while a `hydrateParkedSteps` query is in flight — one
 * collector per in-flight hydration, so overlapping hydrations (two
 * projects, or a retry) each see exactly the clears that raced THEM.
 *
 * The seed's byIssue dedupe cannot catch these: `clearIssue` (step:launch,
 * issue:deleted) deletes the key outright, so an event landing between
 * the IPC resolve and the seed loop leaves no trace, and the seed would
 * resurrect a park the backend no longer holds — a ghost overlay whose
 * confirm would fire against nothing. */
const inFlightHydrationClears = new Set<Set<string>>();

function noteClearedWhileHydrating(issueId: string): void {
  for (const cleared of inFlightHydrationClears) cleared.add(issueId);
}

export const useSteps = create<StepsState>((set) => ({
  byIssue: {},
  hydrated: {},
  error: null,
  launchingByIssue: {},

  setError: (message) => set({ error: message }),

  beginDispatch: (issueId) =>
    set((s) => ({
      launchingByIssue: { ...s.launchingByIssue, [issueId]: {} },
    })),

  noteLaunch: (issueId, taskId) =>
    set((s) => ({
      launchingByIssue: { ...s.launchingByIssue, [issueId]: { taskId } },
    })),

  endLaunch: (issueId) =>
    set((s) => {
      if (!(issueId in s.launchingByIssue)) return s;
      const launchingByIssue = { ...s.launchingByIssue };
      delete launchingByIssue[issueId];
      return { launchingByIssue };
    }),

  notePark: (issueId, columnId, agent) =>
    set((s) => ({
      byIssue: {
        ...s.byIssue,
        [issueId]: {
          queuedColumnId: columnId,
          queuedReason: agent?.reason ?? "confirm",
          queuedProvider: agent?.provider,
          queuedModel: agent?.model ?? null,
          queuedEffort: agent?.effort ?? null,
        },
      },
    })),

  clearPark: (issueId) =>
    set((s) => {
      noteClearedWhileHydrating(issueId);
      const flags = s.byIssue[issueId];
      if (!flags?.queuedColumnId) return s;
      const next = { ...flags };
      delete next.queuedColumnId;
      delete next.queuedReason;
      delete next.queuedProvider;
      delete next.queuedModel;
      delete next.queuedEffort;
      return { byIssue: { ...s.byIssue, [issueId]: next } };
    }),

  clearIssue: (issueId) =>
    set((s) => {
      noteClearedWhileHydrating(issueId);
      if (!(issueId in s.byIssue)) return s;
      const byIssue = { ...s.byIssue };
      delete byIssue[issueId];
      return { byIssue };
    }),
}));

/** Re-seeds a project's parked-step state from `step_parked_list` (E18-09
 * rehydration). Parks live in the backend's in-memory registry; step:*
 * events cover live sessions only, so a webview reload loses the queued
 * dot and the confirm overlay until this runs. Once per project per
 * webview lifetime (a failure un-marks so the next mount retries).
 *
 * Dedupe against events: an issue that ALREADY has flags is skipped —
 * the event stream is fresher than the query snapshot (a `step:queued`
 * that raced in must not be overwritten, and a park an event announced
 * is the same park this query would announce, so seeding it twice would
 * only re-render the same state). `step:queue_cleared` still clears
 * seeded parks through the normal reducer. */
export async function hydrateParkedSteps(projectId: string): Promise<void> {
  const store = useSteps.getState();
  if (store.hydrated[projectId]) return;
  useSteps.setState((s) => ({ hydrated: { ...s.hydrated, [projectId]: true } }));
  // Tombstones for the query's flight window: a step:launch or
  // issue:deleted arriving between the IPC call and the seed loop DELETES
  // the byIssue key, so the dedupe below would not see it — the collector
  // does, and the seed skips those issues instead of resurrecting a park
  // the backend has already consumed.
  const clearedInFlight = new Set<string>();
  inFlightHydrationClears.add(clearedInFlight);
  try {
    const parks = await stepParkedList(projectId);
    useSteps.setState((s) => {
      const byIssue = { ...s.byIssue };
      for (const park of parks) {
        if (byIssue[park.issueId]) continue; // an event already spoke
        if (clearedInFlight.has(park.issueId)) continue; // cleared mid-flight
        byIssue[park.issueId] = {
          queuedColumnId: park.columnId,
          queuedReason: park.reason,
          queuedProvider: park.provider,
          queuedModel: park.model,
          queuedEffort: park.effort,
        };
      }
      return { byIssue };
    });
  } catch {
    // Quiet like the board's other fetches; retry on the next mount.
    useSteps.setState((s) => {
      const hydrated = { ...s.hydrated };
      delete hydrated[projectId];
      return { hydrated };
    });
  } finally {
    inFlightHydrationClears.delete(clearedInFlight);
  }
}

/** Focus a task by id — the launch directive only carries the id, and a
 * freshly provisioned task may not be in the sidebar cache yet. */
function focusTask(taskId: string): void {
  const sidebar = useSidebar.getState();
  const task = Object.values(sidebar.tasksByProject)
    .flat()
    .find((t) => t.id === taskId);
  if (task) sidebar.switchToTask(task);
  else sidebar.selectTask(taskId);
}

/** Navigation is scoped to the project you are LOOKING AT. The launch
 * listener is app-lifetime and unfiltered (it has to be — settle chains
 * fire while the board is unmounted), so an automatic launch in ANOTHER
 * project used to drag the selection across: you were on ade's board and
 * landed in a steadEHR task. The agent still starts either way; only the
 * navigation is withheld. */
function focusIfCurrentProject(launch: StepLaunchDirective): void {
  if (useSidebar.getState().selectedProjectId !== launch.projectId) return;
  focusTask(launch.taskId);
}

/** The launch payload as it rides the event. */
export interface StepLaunchDirective {
  projectId: string;
  taskId: string;
  prompt: string;
  provider: string;
  reattached: boolean;
}

/** Carries out a launch directive: focus when the engine says the card
 * merely re-entered its own column, otherwise open a session and
 * bracket-paste the step's prompt.
 *
 * There is deliberately NO live-agent probe here. It used to be THE
 * re-entry guard (ADR-0033: `terminal_open_agent` reattaches, so
 * "open then paste" silently became "type into the running agent"), but a
 * guard whose only verb is `return` turns a launch the backend recorded
 * into a prompt nobody ran — and the old session's exit then settled the
 * step that never started. The capacity check moved into the engine,
 * which can PARK instead of drop (`ParkReason.AgentBusy`), so a directive
 * that reaches this function is one the backend has already established
 * can run. */
export async function runLaunchDirective(
  launch: StepLaunchDirective,
): Promise<void> {
  try {
    // The engine says so itself (empty prompt = reattach semantics).
    if (launch.reattached || !launch.prompt) {
      focusIfCurrentProject(launch);
      return;
    }
    const terminalId = await terminalOpenAgent(launch.taskId, launch.provider, 24, 80);
    useScripts.getState().noteAgentSpawn(launch.taskId, terminalId);
    // Navigate BEFORE the paste: the task view + terminal tab must be on
    // screen while the agent starts, not materialize after it is already
    // running. The terminal must exist first so ensureTabs discovers it.
    focusIfCurrentProject(launch);
    await waitForTerminalReady(terminalId);
    await terminalWrite(terminalId, `\u001b[200~${launch.prompt}\u001b[201~\r`);
  } catch (e) {
    useSteps.getState().setError(String(e));
  }
}

/** Wires the step engine into the app for its whole lifetime (App.tsx).
 * Returns the unsubscribe, as the other store wirings do. */
export function wireStepEvents(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void onFartcodeEvent((event: FartcodeEvent) => {
    const steps = useSteps.getState();
    if (event.type === "issue:deleted") {
      steps.clearIssue(event.id);
      return;
    }
    if (event.type === "step:launch") {
      // A launch supersedes every derived state the card carried.
      steps.clearIssue(event.issueId);
      steps.noteLaunch(event.issueId, event.taskId);
      // Reveal the provisioned task NOW: it keeps the issue title as its
      // name, no task:renamed is coming, and the sidebar's pasted-prompt
      // skeleton (long names) would otherwise hide the row for its whole
      // 10s cap — which read as "not in Running until the agent ran".
      useSidebar.getState().clearTitlePending(event.taskId);
      // The dispatch stays visible ("starting") until the directive
      // finishes — prompt pasted, reattached/focused, or refused.
      void runLaunchDirective({
        projectId: event.projectId,
        taskId: event.taskId,
        prompt: event.prompt,
        provider: event.provider,
        reattached: event.reattached,
      }).finally(() => useSteps.getState().endLaunch(event.issueId));
      return;
    }
    if (event.type === "step:queued") {
      steps.notePark(event.issueId, event.columnId, {
        provider: event.provider,
        model: event.model,
        effort: event.effort,
        reason: event.reason,
      });
      return;
    }
    if (event.type === "step:queue_cleared") {
      const flags = useSteps.getState().byIssue[event.issueId];
      if (flags?.queuedColumnId === event.columnId) steps.clearPark(event.issueId);
      return;
    }
    if (event.type === "step:settled") {
      useSteps.setState((s) => ({
        byIssue: {
          ...s.byIssue,
          [event.issueId]: {
            // A chain hold rides in right after its StepSettled — keep it.
            ...s.byIssue[event.issueId],
            settledColumnId: event.columnId,
            queuedColumnId: undefined,
            queuedReason: undefined,
            queuedProvider: undefined,
            queuedModel: undefined,
            queuedEffort: undefined,
          },
        },
      }));
      // The agent exited — refresh the card's live-session truth.
      void useScripts.getState().hydrate(event.taskId);
    }
    if (event.type === "step:chain_held") {
      useSteps.setState((s) => ({
        byIssue: {
          ...s.byIssue,
          [event.issueId]: {
            ...s.byIssue[event.issueId],
            heldColumnId: event.columnId,
            holdReason: event.reason,
          },
        },
      }));
    }
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
