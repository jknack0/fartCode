// Board keyboard walk and narrow-mode gating (E18-07 fix round).
//
// The walk is the interesting one: §8b promises "h/l walks EVERY column"
// and the footer advertises it, but landing on an empty column used to
// null the card focus and the next press then restarted the scan from the
// left, so any column behind two consecutive empties had no keyboard path
// at all. The fixture deliberately has interior empty columns.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";

vi.mock("../../lib/tauri", () => ({
  issueList: vi.fn(() => Promise.resolve([])),
  issueCreate: vi.fn(),
  issueMove: vi.fn(() => Promise.resolve()),
  issueEnterColumn: vi.fn(() => Promise.resolve({ step: "inert", issue: null, launch: null })),
  issueImportGithub: vi.fn(),
  projectGithubIssues: vi.fn(() => Promise.resolve([])),
  gitCommitState: vi.fn(() => Promise.resolve({ branch: null })),
  stepConfirm: vi.fn(() => Promise.resolve({ step: "launched", issue: null, launch: null })),
  stepParkedList: vi.fn(() => Promise.resolve([])),
  columnList: vi.fn(() => Promise.resolve([])),
  hostDependencyList: vi.fn(() => Promise.resolve([])),
  hostDependencyRegistrySummary: vi.fn(() => Promise.resolve(null)),
  hostDependencyInstall: vi.fn(),
  hostDependencyUpdate: vi.fn(),
  terminalListForTask: vi.fn(() => Promise.resolve([])),
  terminalOpenLifecycle: vi.fn(),
  terminalOpenAgent: vi.fn(),
  terminalWrite: vi.fn(),
  onTerminalExited: vi.fn(() => Promise.resolve(() => {})),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  // #74: the board reads dossier consent on mount. Already answered, so
  // these fixtures never meet the consent card (DossierConsent.test.tsx
  // owns that path).
  getProjectSettings: vi.fn(() => Promise.resolve({ featureDossiers: true })),
  updateProjectSettings: vi.fn((_p: string, s: unknown) => Promise.resolve(s)),
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

vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn() }));

import BoardView from "./BoardView";
import { columnList, issueList, stepParkedList } from "../../lib/tauri";
import type { BoardColumnDto, IssueDto } from "../../lib/tauri";
import { useColumns } from "../../store/columns";
import { useSteps } from "../../store/steps";
import { useUi } from "../../store/ui";

function column(over: Partial<BoardColumnDto> & { id: string; name: string; position: number }): BoardColumnDto {
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

function issue(id: string, columnId: string, position: number): IssueDto {
  return {
    id,
    projectId: "p1",
    title: id,
    body: null,
    acceptance: [],
    lane: "backlog",
    position,
    provider: null,
    model: null,
    prdPath: null,
    prdSection: null,
    dossierPath: null,
    linkedTaskId: null,
    externalRef: null,
    columnId,
    blocked: false,
    blockers: [],
    createdAt: null,
    updatedAt: null,
  };
}

// Backlog(2) · Ready(0) · Quick(0) · In Progress(1) · In Review(0) · Done(0)
// — the seeded template on a typical board. Two consecutive empties sit
// between the populated columns in both directions.
const COLUMNS = [
  column({ id: "c-backlog", name: "Backlog", position: 0, isLanding: true }),
  column({ id: "c-ready", name: "Ready", position: 1 }),
  column({ id: "c-quick", name: "Quick", position: 2 }),
  column({ id: "c-progress", name: "In Progress", position: 3 }),
  column({ id: "c-review", name: "In Review", position: 4 }),
  column({ id: "c-done", name: "Done", position: 5, countsAsDone: true }),
];

const ISSUES = [
  issue("a", "c-backlog", 0),
  issue("b", "c-backlog", 1),
  issue("p", "c-progress", 0),
];

/** The column the cursor is on, read off the narrow strip's active entry. */
function activeStripColumn(): string | null {
  return document.querySelector(".board-strip-entry[data-active]")?.textContent?.trim() ?? null;
}

/** One keypress, committed. Each real keystroke is its own task, so the
 * handler always sees the previous one's state; act() reproduces that
 * instead of letting a burst of synchronous dispatches share one closure. */
function press(key: string, shiftKey = false): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true }));
  });
}

function setViewportWidth(width: number): void {
  Object.defineProperty(window, "innerWidth", { value: width, configurable: true, writable: true });
  window.dispatchEvent(new Event("resize"));
}

beforeEach(async () => {
  vi.clearAllMocks();
  vi.mocked(columnList).mockResolvedValue(COLUMNS);
  vi.mocked(issueList).mockResolvedValue(ISSUES);
  useColumns.setState({ byProject: {}, loading: {}, loaded: {}, error: null });
  useSteps.setState({ byIssue: {}, hydrated: {}, error: null });
  useUi.setState({ boardDetailIssueId: null });
  setViewportWidth(1440);
});

/** Renders the board and waits for BOTH fetches — columns decide the
 * shape, issues arrive separately, and a keypress before the cards land
 * would have nothing to focus. */
async function renderBoard() {
  const view = render(<BoardView projectId="p1" />);
  await waitFor(() => expect(screen.getByText("In Progress")).toBeInTheDocument());
  await waitFor(() =>
    expect(document.querySelectorAll(".board-card")).toHaveLength(ISSUES.length),
  );
  return view;
}

describe("h/l walks every column", () => {
  it("steps onto an empty column and keeps going in the same direction", async () => {
    await renderBoard();
    // Focus the first Backlog card.
    press("j");
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );

    press("l"); // → Ready (empty): no card is focused, and that is correct
    await waitFor(() => expect(document.querySelector(".board-card.focused")).toBeNull());

    press("l"); // → Quick (empty), NOT backwards to Backlog
    press("l"); // → In Progress, which has a card
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("p"),
    );
  });

  it("reaches the last column across two consecutive empty ones", async () => {
    await renderBoard();
    press("j"); // focus a Backlog card
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );
    // Backlog → Ready → Quick → In Progress → In Review → Done.
    for (let i = 0; i < 5; i++) press("l");

    setViewportWidth(800); // the strip is where the cursor is visible
    await waitFor(() => expect(activeStripColumn()).toMatch(/^done/));
  });

  it("walks back left the same way", async () => {
    await renderBoard();
    press("j");
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );
    for (let i = 0; i < 3; i++) press("l"); // → In Progress
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("p"),
    );
    press("h"); // → Quick (empty)
    press("h"); // → Ready (empty), not a jump back to a card
    press("h"); // → Backlog
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );
  });

  it("does not teleport on j/k while the cursor sits on an empty column", async () => {
    await renderBoard();
    press("j");
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );
    press("l"); // → Ready (empty)
    await waitFor(() => expect(document.querySelector(".board-card.focused")).toBeNull());

    press("j");
    press("k");
    // Still nothing focused, and the cursor has not moved back.
    expect(document.querySelector(".board-card.focused")).toBeNull();
    setViewportWidth(800);
    await waitFor(() => expect(activeStripColumn()).toMatch(/^ready/));
  });
});

// §8c binding copy: "#a is blocked by #b, still in <blocker's column>.
// Send to <Column> anyway?" — the blocker's column comes from config
// (blockerColumnName), never a hardcoded phrase.
// E18-09: parks live in the backend's in-memory registry, so the board's
// mount must re-seed them — this pins the wiring (deleting the
// hydrateParkedSteps call from BoardView's mount effect fails here).
describe("parked-step rehydration wiring", () => {
  it("queries step_parked_list for the project on mount", async () => {
    await renderBoard();
    expect(stepParkedList).toHaveBeenCalledWith("p1");
  });
});

describe("blocked confirm copy", () => {
  const BLOCK_COLUMNS = [
    column({ id: "c-backlog", name: "Backlog", position: 0, isLanding: true }),
    column({ id: "c-plan", name: "Plan", position: 1, kind: "agent_step" }),
    column({ id: "c-quick", name: "Quick", position: 2, kind: "agent_step", onEnter: "run" }),
    column({ id: "c-review", name: "In Review", position: 3, kind: "human_gate" }),
  ];

  function blocker(id: string, columnId: string) {
    return { id, title: id, lane: "backlog" as const, columnId, countsAsDone: false };
  }

  async function renderBlockedBoard(blockers: ReturnType<typeof blocker>[]) {
    const blocked: IssueDto = { ...issue("a", "c-backlog", 0), blocked: true, blockers };
    vi.mocked(columnList).mockResolvedValue(BLOCK_COLUMNS);
    vi.mocked(issueList).mockResolvedValue([blocked]);
    render(<BoardView projectId="p1" />);
    await waitFor(() => expect(document.querySelectorAll(".board-card")).toHaveLength(1));
    press("j"); // focus the blocked card
    await waitFor(() =>
      expect(document.querySelector(".board-card.focused")).toHaveTextContent("a"),
    );
    press("L", true); // shift+L: move it onto the Plan step → blocked confirm
    await waitFor(() => expect(document.querySelector(".board-confirm")).not.toBeNull());
    return document.querySelector(".board-confirm")!;
  }

  it("names the blocker's own column, from config", async () => {
    const overlay = await renderBlockedBoard([blocker("b", "c-quick")]);
    expect(overlay.textContent).toContain("is blocked by b, still in Quick. Send to Plan anyway?");
    expect(overlay.textContent).not.toContain("still in progress");
  });

  it("names each blocker's column when they sit in different columns", async () => {
    const overlay = await renderBlockedBoard([
      blocker("b", "c-quick"),
      blocker("c", "c-review"),
    ]);
    expect(overlay.textContent).toContain(
      "is blocked by b (Quick), c (In Review). Send to Plan anyway?",
    );
  });

  it("collapses to one shared column name when every blocker is in it", async () => {
    const overlay = await renderBlockedBoard([
      blocker("b", "c-quick"),
      blocker("c", "c-quick"),
    ]);
    expect(overlay.textContent).toContain(
      "is blocked by b, c, still in Quick. Send to Plan anyway?",
    );
  });
});

describe("narrow mode", () => {
  it("renders every column while the window is wide", async () => {
    await renderBoard();
    expect(document.querySelectorAll(".board-column")).toHaveLength(6);
    expect(document.querySelector(".board-strip")).toBeNull();
  });

  // Finding 3: narrow used to key on the BOARD pane, so opening the 400px
  // card-detail sheet on a laptop collapsed a wide board to one column and
  // took cross-column drag away. The pane shrinking must not do that.
  it("stays wide when the detail sheet takes 400px off the board pane", async () => {
    const { container } = await renderBoard();
    const board = container.querySelector<HTMLElement>(".board")!;
    // Simulate the pane losing the sheet's width.
    Object.defineProperty(board, "clientWidth", { value: 700, configurable: true });
    window.dispatchEvent(new Event("resize"));

    await waitFor(() => expect(document.querySelectorAll(".board-column")).toHaveLength(6));
    expect(document.querySelector(".board-strip")).toBeNull();
  });

  it("collapses to the strip when the window itself is narrow", async () => {
    await renderBoard();
    setViewportWidth(800);

    await waitFor(() => expect(document.querySelector(".board-strip")).not.toBeNull());
    expect(document.querySelectorAll(".board-column")).toHaveLength(1);
    // Every column is still reachable from the strip — it never caps.
    expect(document.querySelectorAll(".board-strip-entry")).toHaveLength(6);
    expect(screen.getByText(/walk\s*every column/)).toBeInTheDocument();
  });
});
