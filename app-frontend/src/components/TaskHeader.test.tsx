// Task-view pipeline context (ADR-0037).
//
// The bug these cover: the task view had zero board awareness, so a step
// that settled in a `hold` column rendered its decision on the BOARD while
// the user sat in the task view watching the agent. Every assertion here
// is about that gap — the crumb saying where the card is, each action
// appearing exactly when it means something, and the agent dot coming
// from a live session rather than the frozen `task.status`.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  // #74: the task view's move/advance paths now await dossier consent, so
  // this fixture's project has already answered — these tests are about
  // the picker, not the gate (DossierConsent.test.tsx owns that).
  getProjectSettings: vi.fn(() => Promise.resolve({ scripts: {}, featureDossiers: true })),
  issueList: vi.fn(() => Promise.resolve([])),
  columnList: vi.fn(() => Promise.resolve([])),
  issueEnterColumn: vi.fn(() =>
    Promise.resolve({ step: "inert", issue: null, launch: null }),
  ),
  stepConfirm: vi.fn(() =>
    Promise.resolve({ step: "launched", issue: null, launch: null }),
  ),
  hostDependencyList: vi.fn(() => Promise.resolve([])),
  hostDependencyRegistrySummary: vi.fn(() => Promise.resolve(null)),
  hostDependencyInstall: vi.fn(),
  hostDependencyUpdate: vi.fn(),
  terminalListForTask: vi.fn(() => Promise.resolve([])),
  terminalOpen: vi.fn(),
  terminalOpenAgent: vi.fn(),
  terminalOpenLifecycle: vi.fn(),
  terminalClose: vi.fn(),
  terminalWrite: vi.fn(),
  terminalTail: vi.fn(() => Promise.resolve("")),
  terminalSurviving: vi.fn(() => Promise.resolve([])),
  onTerminalExited: vi.fn(() => Promise.resolve(() => {})),
  onTerminalOutput: vi.fn(() => Promise.resolve(() => {})),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  onAcpUpdate: vi.fn(() => Promise.resolve(() => {})),
  onAcpTranscript: vi.fn(() => Promise.resolve(() => {})),
  onAcpPermissionRequest: vi.fn(() => Promise.resolve(() => {})),
  acpStart: vi.fn(),
  acpStop: vi.fn(),
  acpCancel: vi.fn(),
  acpHistory: vi.fn(() => Promise.resolve([])),
  acpSendPrompt: vi.fn(),
  acpResolvePermission: vi.fn(),
  listConversations: vi.fn(() => Promise.resolve([])),
  listProjectConversations: vi.fn(() => Promise.resolve([])),
  getOrCreateProjectConversation: vi.fn(),
  createConversation: vi.fn(),
  listProviders: vi.fn(() => Promise.resolve([])),
  listProjects: vi.fn(() => Promise.resolve([])),
  listTasks: vi.fn(() => Promise.resolve([])),
  createTask: vi.fn(),
  createProject: vi.fn(),
  deleteProject: vi.fn(),
  deleteTask: vi.fn(),
  togglePin: vi.fn(),
  projectGitPull: vi.fn(() => Promise.resolve()),
  gitAddRemote: vi.fn(),
  gitCommit: vi.fn(),
  gitCommitState: vi.fn(() => Promise.resolve({ branch: null })),
  gitFetch: vi.fn(),
  gitPublish: vi.fn(),
  gitPull: vi.fn(),
  gitPush: vi.fn(),
  setViewState: vi.fn(() => Promise.resolve()),
  getViewState: vi.fn(() => Promise.resolve(null)),
}));

import TaskHeader from "./TaskHeader";
import { deleteTask } from "../lib/tauri";
import { registerAllCommands } from "../lib/commands";
import { hint } from "../lib/useCommands";
import {
  runAdvanceStep,
  runConfirmStep,
  runMoveToColumn,
  runOpenCardDetail,
} from "../lib/taskPipeline";
import {
  columnList,
  issueEnterColumn,
  issueList,
  stepConfirm,
  type BoardColumnDto,
  type IssueDto,
  type ProjectDto,
  type TaskDto,
} from "../lib/tauri";
import { useColumns } from "../store/columns";
import { useDependencies } from "../store/dependencies";
import { useDossierConsent } from "../store/dossierConsent";
import { useScripts } from "../store/scripts";
import { useSidebar } from "../store/sidebar";
import { useSteps } from "../store/steps";
import { useTaskCard } from "../store/taskCard";
import { useUi } from "../store/ui";

function column(
  over: Partial<BoardColumnDto> & { id: string; name: string; position: number },
): BoardColumnDto {
  return {
    projectId: "p1",
    kind: "shelf",
    countsAsDone: false,
    isLanding: false,
    onEnter: "queue",
    onSettle: "hold",
    advanceTo: null,
    stepPrompt: null,
    stepProvider: null,
    stepModel: null,
    stepEffort: null,
    stepTools: null,
    seedLane: null,
    createdAt: null,
    updatedAt: null,
    ...over,
  };
}

// Backlog · Plan (queue step) · Implement (run step) · Review (gate) · Done.
const COLUMNS: BoardColumnDto[] = [
  column({ id: "c-backlog", name: "Backlog", position: 0, isLanding: true, seedLane: "backlog" }),
  column({
    id: "c-plan",
    name: "Plan",
    position: 1,
    kind: "agent_step",
    onEnter: "queue",
    stepProvider: "claude",
    stepModel: "fable",
    stepEffort: "high",
  }),
  column({
    id: "c-implement",
    name: "Implement",
    position: 2,
    kind: "agent_step",
    onEnter: "run",
    seedLane: "in_progress",
  }),
  column({ id: "c-review", name: "Review", position: 3, kind: "human_gate" }),
  column({ id: "c-done", name: "Done", position: 4, countsAsDone: true, seedLane: "done" }),
];

function issue(over: Partial<IssueDto> = {}): IssueDto {
  return {
    id: "i1",
    projectId: "p1",
    title: "#47 Pipeline context in the task view",
    body: null,
    acceptance: [],
    lane: "in_progress",
    position: 0,
    provider: null,
    model: null,
    prdPath: null,
    prdSection: null,
    dossierPath: null,
    linkedTaskId: "t1",
    externalRef: null,
    columnId: "c-implement",
    blocked: false,
    blockers: [],
    createdAt: null,
    updatedAt: null,
    ...over,
  };
}

const PROJECT: ProjectDto = {
  id: "p1",
  name: "fartCode",
  path: "/Users/dev/fartCode",
  workspaceProvider: "git",
  baseRef: "main",
  repositoryWorkspaceId: null,
  createdAt: null,
  updatedAt: null,
};

const TASK: TaskDto = {
  id: "t1",
  projectId: "p1",
  name: "Pipeline context",
  // Frozen at birth in production — nothing writes it. Every dot
  // assertion below exists because of that.
  status: "in_progress",
  linkedIssue: null,
  archivedAt: null,
  isPinned: false,
  lastInteractedAt: null,
  statusChangedAt: null,
  workspaceId: "w1",
  createdBy: "user",
  type: "task",
};

// The header's buttons run REGISTERED commands (⌘N new task is the point
// of item 4), so the registry has to exist. Chord hints then render in the
// registry's own format — jsdom is not macOS, so they read "Meta+Shift+M",
// not "⌘⇧M". Buttons are therefore addressed by their titles; the label's
// key prefix is asserted once, against the registry, below.
registerAllCommands();

/** Title → the header button, matching how each is rendered. */
const ACTION = {
  advance: (column: string) => `Advance to ${column} (${hint("advance-step")})`,
  dispatch: (column: string) =>
    `Dispatch the step parked in ${column} (${hint("confirm-step")})`,
  move: /^Move this card to another column/,
  card: /^Open the card detail/,
  changes: `Toggle changes panel (${hint("toggle-changes")})`,
  newTask: /^Add a task to this project/,
  deleteTask: /^Delete this task and its worktree/,
};

beforeEach(() => {
  vi.clearAllMocks();
  // The consent gate caches per project for the process lifetime — clear
  // it so each test re-reads the mock rather than a neighbour's answer.
  useDossierConsent.getState().reset();
  vi.mocked(columnList).mockResolvedValue(COLUMNS);
  vi.mocked(issueList).mockResolvedValue([issue()]);
  useSidebar.setState({
    projects: [PROJECT],
    tasksByProject: { p1: [TASK] },
    selectedProjectId: "p1",
    selectedTaskId: "t1",
  });
  useColumns.setState({ byProject: {}, loading: {}, loaded: {}, error: null });
  useTaskCard.setState({
    issuesByProject: {},
    loading: {},
    loaded: {},
    overlay: null,
    error: null,
  });
  useSteps.setState({ byIssue: {}, error: null });
  useScripts.setState({ byTask: {}, agentByTask: {} });
  useDependencies.setState({ deps: [] });
  useUi.setState({
    boardDetailIssueId: null,
    changesOpen: false,
    createTaskTarget: null,
    sidebarVisible: true,
  });
});

/** Renders the header and waits for the board reads (columns + cards) that
 * the crumb and the actions depend on. */
async function renderHeader(taskId = "t1") {
  const view = render(<TaskHeader taskId={taskId} />);
  await waitFor(() => expect(useColumns.getState().loaded.p1).toBe(true));
  await waitFor(() => expect(useTaskCard.getState().loaded.p1).toBe(true));
  return view;
}

const crumbText = () =>
  document.querySelector(".tv-crumb")?.textContent?.trim() ?? "";

function press(key: string): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
}

describe("breadcrumb carries the pipeline position", () => {
  it("reads project / column / ref on a carded task", async () => {
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode / Implement / #47 /"));
    expect(screen.getByText("Pipeline context")).toBeInTheDocument();
  });

  it("resolves the column the board's way — the mirror wins over the lane", async () => {
    // lane says in_progress (which seeds Implement); the mirror points at
    // Plan, a column no lane can name. The mirror is what renders.
    vi.mocked(issueList).mockResolvedValue([issue({ columnId: "c-plan" })]);
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode / Plan / #47 /"));
  });

  it("falls back to the seeded lane column when the card has no mirror", async () => {
    vi.mocked(issueList).mockResolvedValue([issue({ columnId: null })]);
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode / Implement / #47 /"));
  });

  it("omits the ref segment rather than an empty separator", async () => {
    vi.mocked(issueList).mockResolvedValue([
      issue({ title: "Pipeline context", externalRef: null }),
    ]);
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode / Implement /"));
  });

  it("keeps the ad-hoc crumb exactly as it was", async () => {
    vi.mocked(issueList).mockResolvedValue([]); // ⌘N task: no card at all
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode /"));
  });
});

describe("a cardless (⌘N) task has no pipeline", () => {
  it("renders neither a pipeline crumb nor a pipeline action", async () => {
    vi.mocked(issueList).mockResolvedValue([]);
    await renderHeader();
    await waitFor(() => expect(crumbText()).toBe("fartCode /"));
    expect(screen.queryByTitle(/^Advance to /)).toBeNull();
    expect(screen.queryByTitle(/^Dispatch the step parked/)).toBeNull();
    // The always-on entries survive; the pipeline entries stay absent.
    expect(screen.getByTitle(ACTION.changes)).toBeInTheDocument();
    expect(screen.queryByTitle(ACTION.move)).toBeNull();
    expect(screen.queryByTitle(ACTION.card)).toBeNull();
    expect(screen.getByTitle(ACTION.newTask)).toBeInTheDocument();
  });

  it("no-ops safely when a pipeline command fires anyway", async () => {
    vi.mocked(issueList).mockResolvedValue([]);
    await renderHeader();
    act(() => {
      runAdvanceStep();
      runConfirmStep();
      runMoveToColumn();
      runOpenCardDetail();
    });
    expect(issueEnterColumn).not.toHaveBeenCalled();
    expect(stepConfirm).not.toHaveBeenCalled();
    expect(useTaskCard.getState().overlay).toBeNull();
  });
});

describe("advance — visible only when a step settled in a holding column", () => {
  it("stays hidden while nothing has settled", async () => {
    await renderHeader();
    expect(screen.queryByTitle(/^Advance to /)).toBeNull();
    // …and the always-available actions are there.
    expect(screen.getByTitle(ACTION.move)).toBeInTheDocument();
    expect(screen.getByTitle(ACTION.card)).toBeInTheDocument();
  });

  it("stays hidden when the settle belongs to a column the card has left", async () => {
    await renderHeader();
    act(() => useSteps.setState({ byIssue: { i1: { settledColumnId: "c-plan" } } }));
    expect(screen.queryByTitle(/^Advance to /)).toBeNull();
  });

  it("targets the next column by position when no advance_to is set", async () => {
    await renderHeader();
    act(() => useSteps.setState({ byIssue: { i1: { settledColumnId: "c-implement" } } }));
    screen.getByTitle(ACTION.advance("Review")).click();
    await waitFor(() =>
      expect(issueEnterColumn).toHaveBeenCalledWith("i1", "c-review"),
    );
  });

  it("targets advance_to when the column declares one", async () => {
    vi.mocked(columnList).mockResolvedValue(
      COLUMNS.map((c) => (c.id === "c-implement" ? { ...c, advanceTo: "c-done" } : c)),
    );
    await renderHeader();
    act(() => useSteps.setState({ byIssue: { i1: { settledColumnId: "c-implement" } } }));
    screen.getByTitle(ACTION.advance("Done")).click();
    await waitFor(() =>
      expect(issueEnterColumn).toHaveBeenCalledWith("i1", "c-done"),
    );
  });

  it("keeps the chord in the hover title, not the label", async () => {
    await renderHeader();
    act(() => useSteps.setState({ byIssue: { i1: { settledColumnId: "c-implement" } } }));
    const button = screen.getByTitle(ACTION.advance("Review"));
    expect(button.textContent).toBe("advance");
    expect(button.getAttribute("title")).toContain(hint("advance-step"));
    expect(hint("advance-step")).not.toBe("");
  });

  it("brightens the key when the advance lands on a confirm-free spend", async () => {
    // Plan → Implement runs on arrival, and DESIGN.md's rule is that the
    // brighter reading is the warning.
    vi.mocked(issueList).mockResolvedValue([issue({ columnId: "c-plan" })]);
    await renderHeader();
    act(() => useSteps.setState({ byIssue: { i1: { settledColumnId: "c-plan" } } }));
    expect(screen.getByTitle(ACTION.advance("Implement"))).toHaveAttribute(
      "data-tone",
      "run",
    );
  });
});

describe("dispatch — a parked step names its spend before it fires", () => {
  const park = () =>
    act(() =>
      useSteps.setState({
        byIssue: {
          i1: {
            queuedColumnId: "c-plan",
            queuedProvider: "claude",
            queuedModel: "fable",
            queuedEffort: "high",
          },
        },
      }),
    );

  it("stays hidden while nothing is parked", async () => {
    await renderHeader();
    expect(screen.queryByTitle(/^Dispatch the step parked/)).toBeNull();
  });

  it("names provider · model · effort and does not fire on the press", async () => {
    await renderHeader();
    park();
    screen.getByTitle(ACTION.dispatch("Plan")).click();
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "Plan runs claude · fable · high — queue on #47. Dispatch?",
    );
    expect(stepConfirm).not.toHaveBeenCalled();
  });

  it("fires on ↵ from the overlay", async () => {
    await renderHeader();
    park();
    screen.getByTitle(ACTION.dispatch("Plan")).click();
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
    press("Enter");
    await waitFor(() => expect(stepConfirm).toHaveBeenCalledWith("i1"));
  });

  it("esc leaves the step parked", async () => {
    await renderHeader();
    park();
    screen.getByTitle(ACTION.dispatch("Plan")).click();
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
    press("Escape");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(stepConfirm).not.toHaveBeenCalled();
    expect(useSteps.getState().byIssue.i1.queuedColumnId).toBe("c-plan");
  });
});

describe("move — the key-first column picker", () => {
  async function openPicker() {
    await renderHeader();
    screen.getByTitle(ACTION.move).click();
    await waitFor(() => expect(screen.getByRole("dialog")).toBeInTheDocument());
  }

  const focused = () =>
    document.querySelector(".tv-pick-row[data-focused] .tv-pick-name")?.textContent ?? "";

  it("lists every column and opens on the card's own", async () => {
    await openPicker();
    expect(document.querySelectorAll(".tv-pick-row")).toHaveLength(COLUMNS.length);
    expect(focused()).toContain("Implement");
    expect(focused()).toContain("here");
  });

  it("walks with j/k and moves on ↵", async () => {
    await openPicker();
    press("j"); // Implement → Review
    expect(focused()).toContain("Review");
    press("j"); // → Done
    press("k"); // back to Review
    expect(focused()).toContain("Review");
    press("Enter");
    await waitFor(() =>
      expect(issueEnterColumn).toHaveBeenCalledWith("i1", "c-review"),
    );
  });

  it("does not walk off either end", async () => {
    await openPicker();
    for (let i = 0; i < 6; i++) press("k");
    expect(focused()).toContain("Backlog");
    for (let i = 0; i < 9; i++) press("j");
    expect(focused()).toContain("Done");
  });

  it("swaps the verb for a run-mode step, which spends without asking again", async () => {
    await openPicker();
    press("k"); // Implement → Plan (queue-mode)
    expect(screen.getByRole("dialog")).toHaveTextContent("↵ move to Plan");
    press("j"); // back to Implement (run-mode)
    expect(screen.getByRole("dialog")).toHaveTextContent("↵ dispatch Implement");
  });

  it("carries the board's spend brightness onto every row", async () => {
    await openPicker();
    const tones = Array.from(document.querySelectorAll(".tv-pick-sub")).map((n) =>
      n.getAttribute("data-tone"),
    );
    expect(tones).toEqual(["kind", "queue", "run", "kind", "kind"]);
  });

  it("esc cancels without moving anything", async () => {
    await openPicker();
    press("Escape");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(issueEnterColumn).not.toHaveBeenCalled();
  });

  it("chains straight into the confirm when the move parked a step", async () => {
    vi.mocked(issueEnterColumn).mockResolvedValue({
      step: "queued",
      issue: issue({ columnId: "c-plan" }),
      launch: null,
    });
    await openPicker();
    press("k"); // Plan — queue mode
    press("Enter");
    act(() =>
      useSteps.setState({ byIssue: { i1: { queuedColumnId: "c-plan" } } }),
    );
    await waitFor(() => expect(useTaskCard.getState().overlay).toBe("confirm"));
  });
});

describe("open card detail", () => {
  it("routes to the sheet on the card", async () => {
    await renderHeader();
    screen.getByTitle(ACTION.card).click();
    await waitFor(() => expect(useUi.getState().boardDetailIssueId).toBe("i1"));
    expect(useUi.getState().changesOpen).toBe(true);
    // The detail lives in the project view's right slot, so reading the
    // ticket leaves the task view — nothing is torn down.
    expect(useSidebar.getState().selectedTaskId).toBeNull();
  });
});

describe("the agent dot derives from the live session", () => {
  it("reads idle on a task whose status says in_progress but has no agent", async () => {
    await renderHeader();
    expect(document.querySelector(".tv-header-id .status-dot")).toHaveClass("tv-dot-idle");
  });

  it("goes running when the scripts store sees a live agent terminal", async () => {
    await renderHeader();
    act(() =>
      useScripts.setState({
        agentByTask: { t1: { ids: ["term-1"], running: true, exitedAt: null } },
      }),
    );
    expect(document.querySelector(".tv-header-id .status-dot")).toHaveClass(
      "status-in_progress",
    );
  });

  it("goes back to idle when that terminal exits", async () => {
    await renderHeader();
    act(() =>
      useScripts.setState({
        agentByTask: { t1: { ids: ["term-1"], running: true, exitedAt: null } },
      }),
    );
    act(() =>
      useScripts.setState({
        agentByTask: { t1: { ids: [], running: false, exitedAt: Date.now() } },
      }),
    );
    expect(document.querySelector(".tv-header-id .status-dot")).toHaveClass("tv-dot-idle");
  });
});

describe("reachable delete", () => {
  it("opens the confirm for this task rather than deleting it", async () => {
    await renderHeader();
    screen.getByTitle(ACTION.deleteTask).click();
    // The header opens the SAME confirm the ⌘⌫ command opens — nothing is
    // destroyed on the press, and the modal is what itemizes the worktree.
    await waitFor(() =>
      expect(useUi.getState().deleteTaskTarget).toEqual({ projectId: "p1", taskId: "t1" }),
    );
    expect(vi.mocked(deleteTask)).not.toHaveBeenCalled();
  });
});

describe("always-reachable new task", () => {
  it("shows the ⌘N entry even with the flyout collapsed", async () => {
    useUi.setState({ sidebarVisible: false });
    await renderHeader();
    const button = screen.getByTitle(ACTION.newTask);
    button.click();
    // The existing add-task command is what runs — the header duplicates
    // no creation logic.
    await waitFor(() => expect(useUi.getState().createTaskTarget).toBe("p1"));
  });
});
