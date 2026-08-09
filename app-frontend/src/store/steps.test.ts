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
  terminalListForTask,
  terminalOpenAgent,
  terminalWrite,
} from "../lib/tauri";
import { useScripts } from "./scripts";
import { useSidebar } from "./sidebar";
import { runLaunchDirective, useSteps, wireStepEvents } from "./steps";

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
  useSteps.setState({ byIssue: {}, error: null });
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
