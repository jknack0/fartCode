// Step engine plumbing (E18-07 fix round): the launch directive and the
// per-issue flags. Everything here is deliberately exercised WITHOUT a
// mounted board — that is the point of the fix. Carrying out a launch
// navigates to the task view, which unmounts BoardView, so a component
// that owned this subscription would tear it down at the exact moment a
// settle-chained launch arrived.

import { describe, it, expect, beforeEach, vi } from "vitest";

const listeners: ((event: unknown) => void)[] = [];

vi.mock("../lib/tauri", () => ({
  onFartcodeEvent: vi.fn((cb: (event: unknown) => void) => {
    listeners.push(cb);
    return Promise.resolve(() => {
      const i = listeners.indexOf(cb);
      if (i >= 0) listeners.splice(i, 1);
    });
  }),
  stepParkedList: vi.fn(() => Promise.resolve([])),
  terminalListForTask: vi.fn(() => Promise.resolve([])),
  terminalOpenAgent: vi.fn(() => Promise.resolve("term-1")),
  terminalWrite: vi.fn(() => Promise.resolve()),
  terminalOpenLifecycle: vi.fn(() => Promise.resolve("term-lc")),
  onTerminalExited: vi.fn(() => Promise.resolve(() => {})),
  listProjects: vi.fn(() => Promise.resolve([])),
  listTasks: vi.fn(() => Promise.resolve([])),
  createTask: vi.fn(),
  createProject: vi.fn(),
  deleteProject: vi.fn(),
  deleteTask: vi.fn(),
  togglePin: vi.fn(),
  projectGitPull: vi.fn(() => Promise.resolve()),
  setViewState: vi.fn(() => Promise.resolve()),
  getViewState: vi.fn(() => Promise.resolve(null)),
}));

import {
  onFartcodeEvent,
  stepParkedList,
  terminalListForTask,
  terminalOpenAgent,
  terminalWrite,
} from "../lib/tauri";
import { useScripts } from "./scripts";
import { useSidebar } from "./sidebar";
import { hydrateParkedSteps, runLaunchDirective, useSteps, wireStepEvents } from "./steps";

/** Delivers an event to every wired listener, as the Tauri channel would. */
function emit(event: Record<string, unknown>): void {
  for (const cb of [...listeners]) cb(event);
}

/** Lets the directive's awaits settle. */
const flush = () => new Promise((r) => setTimeout(r, 0));

const LAUNCH = {
  type: "step:launch",
  issueId: "iss-1",
  projectId: "p1",
  columnId: "col-progress",
  taskId: "task-1",
  prompt: "implement the thing",
  provider: "claude",
  model: null,
  effort: null,
  reattached: false,
};

let unwire: (() => void) | null = null;

// The suite's `restoreMocks` clears factory implementations between
// tests, so every mock is (re)established here rather than once.
beforeEach(() => {
  unwire?.();
  unwire = null;
  listeners.length = 0;
  vi.clearAllMocks();
  vi.mocked(onFartcodeEvent).mockImplementation((cb) => {
    listeners.push(cb as (event: unknown) => void);
    return Promise.resolve(() => {
      const i = listeners.indexOf(cb as (event: unknown) => void);
      if (i >= 0) listeners.splice(i, 1);
    });
  });
  vi.mocked(terminalListForTask).mockResolvedValue([]);
  vi.mocked(terminalOpenAgent).mockResolvedValue("term-1");
  vi.mocked(terminalWrite).mockResolvedValue(undefined);
  vi.mocked(stepParkedList).mockResolvedValue([]);
  useSteps.setState({ byIssue: {}, hydrated: {}, error: null });
  useScripts.setState({ byTask: {}, agentByTask: {} });
  useSidebar.setState({ tasksByProject: {}, selectedTaskId: null, selectedProjectId: null });
});

describe("the launch directive", () => {
  it("opens a session and pastes the step prompt when nothing is running", async () => {
    await runLaunchDirective({
      taskId: "task-1",
      prompt: "implement the thing",
      provider: "claude",
      reattached: false,
    });

    expect(terminalOpenAgent).toHaveBeenCalledWith("task-1", "claude", 24, 80);
    expect(terminalWrite).toHaveBeenCalledTimes(1);
    expect(vi.mocked(terminalWrite).mock.calls[0][1]).toContain("implement the thing");
    expect(useSidebar.getState().selectedTaskId).toBe("task-1");
  });

  // THE regression this round exists for: a rework drag (In Review → In
  // Progress) reports reattached:false and carries a full dispatch packet,
  // but terminal_open_agent hands back the RUNNING pty (ADR-0033), so the
  // paste lands in the middle of the agent's turn.
  it("never writes to a task that already has a live agent", async () => {
    vi.mocked(terminalListForTask).mockResolvedValue([
      { id: "term-live", agent: "claude", kind: "agent", scriptType: null, running: true, exitCode: null },
    ]);

    await runLaunchDirective({
      taskId: "task-1",
      prompt: "implement the thing",
      provider: "claude",
      reattached: false, // the engine's own answer is not to be trusted here
    });

    expect(terminalWrite).not.toHaveBeenCalled();
    expect(terminalOpenAgent).not.toHaveBeenCalled();
    expect(useSidebar.getState().selectedTaskId).toBe("task-1");
  });

  it("takes the scripts store's word for liveness without an extra round trip", async () => {
    useScripts.setState({
      agentByTask: { "task-1": { ids: ["term-live"], running: true, exitedAt: null } },
    });

    await runLaunchDirective({
      taskId: "task-1",
      prompt: "implement the thing",
      provider: "claude",
      reattached: false,
    });

    expect(terminalListForTask).not.toHaveBeenCalled();
    expect(terminalWrite).not.toHaveBeenCalled();
  });

  it("focuses without writing when the engine says it reattached", async () => {
    await runLaunchDirective({
      taskId: "task-1",
      prompt: "",
      provider: "claude",
      reattached: true,
    });

    expect(terminalOpenAgent).not.toHaveBeenCalled();
    expect(terminalWrite).not.toHaveBeenCalled();
    expect(useSidebar.getState().selectedTaskId).toBe("task-1");
  });

  it("surfaces a directive that could not be carried out", async () => {
    vi.mocked(terminalOpenAgent).mockRejectedValue(new Error("agent not installed: claude"));

    await runLaunchDirective({
      taskId: "task-1",
      prompt: "go",
      provider: "claude",
      reattached: false,
    });

    expect(useSteps.getState().error).toContain("agent not installed");
  });
});

describe("wireStepEvents", () => {
  beforeEach(() => {
    unwire = wireStepEvents();
  });

  it("carries out a launch with no board mounted", async () => {
    emit(LAUNCH);
    await flush();

    expect(terminalOpenAgent).toHaveBeenCalledWith("task-1", "claude", 24, 80);
    expect(terminalWrite).toHaveBeenCalledTimes(1);
  });

  // Findings 4 and 8: the old 4s wall-clock claim refused a second launch
  // for the same issue+column, while the backend had performed it in full
  // and marked it delivered — a user-initiated dispatch became a silent
  // no-op. There is no window any more, so a retry always acts.
  it("does not swallow a retry after a failed launch", async () => {
    vi.mocked(terminalOpenAgent).mockRejectedValueOnce(new Error("pty refused"));

    emit(LAUNCH);
    await flush();
    expect(useSteps.getState().error).toContain("pty refused");
    expect(terminalWrite).not.toHaveBeenCalled();

    // Same issue, same column, immediately after — the user re-dragging.
    emit(LAUNCH);
    await flush();

    expect(terminalOpenAgent).toHaveBeenCalledTimes(2);
    expect(terminalWrite).toHaveBeenCalledTimes(1);
  });

  // Finding 7: a settle chain launches step B while the user is watching
  // step A in the task view, i.e. with the board unmounted.
  it("carries out a settle-chained launch into another column", async () => {
    emit(LAUNCH);
    await flush();
    // The first launch spawned an agent; the chain runs in the same task.
    useScripts.setState({ agentByTask: {} });

    emit({ ...LAUNCH, columnId: "col-review", prompt: "review the thing" });
    await flush();

    expect(terminalOpenAgent).toHaveBeenCalledTimes(2);
    expect(vi.mocked(terminalWrite).mock.calls[1][1]).toContain("review the thing");
  });

  // Finding 10: the flags used to be component state, wiped by the very
  // navigation the launch performs.
  it("keeps step-done and queued flags across everything a board unmount would clear", async () => {
    emit({
      type: "step:settled",
      issueId: "iss-1",
      projectId: "p1",
      columnId: "col-progress",
      taskId: "task-1",
    });
    expect(useSteps.getState().byIssue["iss-1"]?.settledColumnId).toBe("col-progress");

    emit({
      type: "step:queued",
      issueId: "iss-2",
      projectId: "p1",
      columnId: "col-plan",
      provider: "fable",
      model: "opus",
      effort: "high",
    });
    const parked = useSteps.getState().byIssue["iss-2"];
    expect(parked?.queuedColumnId).toBe("col-plan");
    expect(parked?.queuedProvider).toBe("fable");

    // Nothing here is component-scoped, so both survive.
    expect(useSteps.getState().byIssue["iss-1"]?.settledColumnId).toBe("col-progress");
  });

  it("clears a park when the engine says it was cleared", () => {
    emit({
      type: "step:queued",
      issueId: "iss-2",
      projectId: "p1",
      columnId: "col-plan",
      provider: "fable",
      model: null,
      effort: null,
    });
    emit({
      type: "step:queue_cleared",
      issueId: "iss-2",
      projectId: "p1",
      columnId: "col-plan",
    });

    expect(useSteps.getState().byIssue["iss-2"]?.queuedColumnId).toBeUndefined();
  });

  // E18-09: parks live in the backend's in-memory registry, so a webview
  // reload loses them until step_parked_list re-seeds the store.
  it("rehydrates parked steps from the query after a reload", async () => {
    vi.mocked(stepParkedList).mockResolvedValue([
      {
        issueId: "iss-3",
        projectId: "p1",
        columnId: "col-plan",
        provider: "fable",
        model: "opus",
        effort: "high",
      },
    ]);

    await hydrateParkedSteps("p1");

    const flags = useSteps.getState().byIssue["iss-3"];
    expect(flags?.queuedColumnId).toBe("col-plan");
    expect(flags?.queuedProvider).toBe("fable");
    expect(flags?.queuedModel).toBe("opus");
    expect(flags?.queuedEffort).toBe("high");

    // Once per project per webview lifetime.
    await hydrateParkedSteps("p1");
    expect(stepParkedList).toHaveBeenCalledTimes(1);
  });

  it("does not double-park or clobber when the event already announced the park", async () => {
    // The event lands first (fresher than the query snapshot)…
    emit({
      type: "step:queued",
      issueId: "iss-3",
      projectId: "p1",
      columnId: "col-review",
      provider: "claude",
      model: null,
      effort: null,
    });
    // …and the query answers with a stale snapshot of the same issue.
    vi.mocked(stepParkedList).mockResolvedValue([
      {
        issueId: "iss-3",
        projectId: "p1",
        columnId: "col-plan",
        provider: "fable",
        model: "opus",
        effort: "high",
      },
    ]);

    await hydrateParkedSteps("p1");

    const flags = useSteps.getState().byIssue["iss-3"];
    expect(flags?.queuedColumnId).toBe("col-review"); // the event's word stands
    expect(flags?.queuedProvider).toBe("claude");
  });

  it("a seeded park is still cleared by step:queue_cleared", async () => {
    vi.mocked(stepParkedList).mockResolvedValue([
      {
        issueId: "iss-3",
        projectId: "p1",
        columnId: "col-plan",
        provider: "fable",
        model: null,
        effort: null,
      },
    ]);
    await hydrateParkedSteps("p1");
    expect(useSteps.getState().byIssue["iss-3"]?.queuedColumnId).toBe("col-plan");

    emit({
      type: "step:queue_cleared",
      issueId: "iss-3",
      projectId: "p1",
      columnId: "col-plan",
    });
    expect(useSteps.getState().byIssue["iss-3"]?.queuedColumnId).toBeUndefined();
  });

  // Fix round: clearIssue DELETES the byIssue key, so an event landing
  // between the query's IPC resolve and the seed loop left no trace for
  // the byIssue dedupe — the seed resurrected a park the backend had
  // already consumed (ghost overlay whose confirm fires against nothing).
  it("does not resurrect a park cleared while the query was in flight", async () => {
    let resolveQuery!: (parks: unknown[]) => void;
    vi.mocked(stepParkedList).mockReturnValue(
      new Promise((r) => {
        resolveQuery = r as (parks: unknown[]) => void;
      }) as ReturnType<typeof stepParkedList>,
    );

    const hydration = hydrateParkedSteps("p1");
    // The parked step launches (step:launch clears via clearIssue) while
    // the query is still on the wire…
    emit({ ...LAUNCH, issueId: "iss-3" });
    await flush();
    expect(useSteps.getState().byIssue["iss-3"]).toBeUndefined();

    // …and the query then answers with its stale pre-launch snapshot.
    resolveQuery([
      {
        issueId: "iss-3",
        projectId: "p1",
        columnId: "col-plan",
        provider: "fable",
        model: null,
        effort: null,
      },
    ]);
    await hydration;

    expect(useSteps.getState().byIssue["iss-3"]).toBeUndefined();
  });

  it("an in-flight clear on one issue does not block seeding the others", async () => {
    let resolveQuery!: (parks: unknown[]) => void;
    vi.mocked(stepParkedList).mockReturnValue(
      new Promise((r) => {
        resolveQuery = r as (parks: unknown[]) => void;
      }) as ReturnType<typeof stepParkedList>,
    );

    const hydration = hydrateParkedSteps("p1");
    emit({ type: "issue:deleted", id: "iss-3" });
    const park = (issueId: string) => ({
      issueId,
      projectId: "p1",
      columnId: "col-plan",
      provider: "fable",
      model: null,
      effort: null,
    });
    resolveQuery([park("iss-3"), park("iss-4")]);
    await hydration;

    expect(useSteps.getState().byIssue["iss-3"]).toBeUndefined();
    expect(useSteps.getState().byIssue["iss-4"]?.queuedColumnId).toBe("col-plan");
  });

  it("a failed hydration retries on the next call", async () => {
    vi.mocked(stepParkedList).mockRejectedValueOnce(new Error("ipc down"));
    await hydrateParkedSteps("p1");
    expect(useSteps.getState().byIssue).toEqual({});

    vi.mocked(stepParkedList).mockResolvedValue([
      {
        issueId: "iss-3",
        projectId: "p1",
        columnId: "col-plan",
        provider: "fable",
        model: null,
        effort: null,
      },
    ]);
    await hydrateParkedSteps("p1");
    expect(useSteps.getState().byIssue["iss-3"]?.queuedColumnId).toBe("col-plan");
  });

  it("a launch supersedes the card's derived state", async () => {
    emit({
      type: "step:settled",
      issueId: "iss-1",
      projectId: "p1",
      columnId: "col-progress",
      taskId: "task-1",
    });
    emit(LAUNCH);
    await flush();

    expect(useSteps.getState().byIssue["iss-1"]).toBeUndefined();
  });
});
