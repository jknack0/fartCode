// First-dispatch dossier consent (#74, handoff v3 §8e).
//
// The load-bearing assertions, in the order they'd bite:
//   1. BOTH answers persist an explicit boolean. A decline that left
//      `featureDossiers` null would fail closed backend-side AND re-ask on
//      the next dispatch — forever. That is the bug this ticket exists to
//      not ship.
//   2. Declining still dispatches. The card gates the repo write, never
//      the agent.
//   3. Consent comes BEFORE the queue confirm, and the entry does not
//      happen until it is answered — the backend reads consent at launch
//      time, so an answer arriving after `issue_enter_column` would miss
//      its own dispatch.
//   4. Shelves and human gates never ask; an answered project never asks
//      again, either way.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, fireEvent, render, waitFor } from "@testing-library/react";

vi.mock("../../lib/tauri", () => ({
  issueList: vi.fn(() => Promise.resolve([])),
  issueCreate: vi.fn(),
  issueMove: vi.fn(() => Promise.resolve()),
  issueEnterColumn: vi.fn(),
  issueImportGithub: vi.fn(),
  projectGithubIssues: vi.fn(() => Promise.resolve([])),
  gitCommitState: vi.fn(() => Promise.resolve({ branch: null })),
  stepConfirm: vi.fn(() => Promise.resolve({ step: "launched", issue: null, launch: null })),
  stepParkedList: vi.fn(() => Promise.resolve([])),
  columnList: vi.fn(() => Promise.resolve([])),
  getProjectSettings: vi.fn(),
  updateProjectSettings: vi.fn(),
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
import { dossierPathFor, dossierSlug } from "./DossierConsent";
import {
  columnList,
  getProjectSettings,
  issueEnterColumn,
  issueList,
  updateProjectSettings,
  type BoardColumnDto,
  type IssueDto,
  type ProjectSettingsDto,
} from "../../lib/tauri";
import { useColumns } from "../../store/columns";
import { useSteps } from "../../store/steps";
import { useUi } from "../../store/ui";

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

function issue(columnId: string): IssueDto {
  return {
    id: "iss_a",
    projectId: "p1",
    title: "Implement OAuth login",
    body: null,
    acceptance: [],
    lane: "backlog",
    position: 0,
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

// Backlog(shelf, landing) · Ready(shelf) · Plan(agent_step, on_enter queue)
// · Review(human gate). One of each kind the gate has to tell apart.
const COLUMNS = [
  column({ id: "c-backlog", name: "Backlog", position: 0, isLanding: true }),
  column({ id: "c-ready", name: "Ready", position: 1 }),
  column({
    id: "c-plan",
    name: "Plan",
    position: 2,
    kind: "agent_step",
    onEnter: "queue",
    stepProvider: "claude",
  }),
  column({ id: "c-review", name: "In Review", position: 3, kind: "human_gate" }),
];

/** Base settings row — anything the pane would round-trip, so a write that
 * dropped a key would be visible in the assertion. */
const SETTINGS: ProjectSettingsDto = {
  tmux: true,
  featureDossiers: null,
  featureLogSeededVersion: 3,
};

function press(key: string, shiftKey = false): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true }));
  });
}

const consentCard = () => document.querySelector(".board-consent");
const confirmCard = () => document.querySelector(".board-confirm:not(.board-consent)");

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(columnList).mockResolvedValue(COLUMNS);
  vi.mocked(getProjectSettings).mockResolvedValue(SETTINGS);
  vi.mocked(updateProjectSettings).mockImplementation((_p, s) => Promise.resolve(s));
  // Entering Plan parks a step, so the queue confirm is the thing consent
  // must come before.
  vi.mocked(issueEnterColumn).mockImplementation((issueId) =>
    Promise.resolve({
      step: "queued",
      issue: { ...issue("c-plan"), id: issueId },
      launch: null,
    }),
  );
  useColumns.setState({ byProject: {}, loading: {}, loaded: {}, error: null });
  useSteps.setState({ byIssue: {}, hydrated: {}, error: null });
  useUi.setState({ boardDetailIssueId: null });
  Object.defineProperty(window, "innerWidth", { value: 1440, configurable: true });
});

/** Renders the board with the card parked in `startColumn`, focuses it,
 * and waits for the consent read to settle so the gate is not racing. */
async function renderBoard(startColumn: string) {
  vi.mocked(issueList).mockResolvedValue([issue(startColumn)]);
  render(<BoardView projectId="p1" />);
  await waitFor(() => expect(document.querySelectorAll(".board-card")).toHaveLength(1));
  await waitFor(() => expect(getProjectSettings).toHaveBeenCalledWith("p1"));
  // Click rather than j/k: it puts BOTH cursors (card and column) on this
  // card wherever it sits, so the fixture can start it in any column.
  fireEvent.click(document.querySelector(".board-card")!);
  await waitFor(() =>
    expect(document.querySelector(".board-card.focused")).not.toBeNull(),
  );
}

/** ⇧L: move the focused card one column right, through the same gates a
 * drag goes through. */
const moveRight = () => press("L", true);

describe("the card", () => {
  it("renders §8e's copy, the real slug, and the key-first footer", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    const card = consentCard()!;

    expect(card.querySelector(".board-consent-body")).toHaveTextContent(
      "This feature will keep a dossier — write the convention files to your repo?",
    );

    const files = Array.from(card.querySelectorAll(".board-consent-files li")).map(
      (li) => li.textContent,
    );
    expect(files).toEqual([
      "docs/features/implement-oauth-login.md",
      ".claude/skills/feature-log/",
      "AGENTS.md · one pointer line",
    ]);

    expect(card.querySelector(".board-consent-note")).toHaveTextContent(
      "provenance-tagged · commits ride the feature branch",
    );

    const foot = card.querySelector(".board-confirm-foot")!;
    expect(foot.textContent).toContain("esc run without memory");
    expect(foot.textContent).toContain("write to repo");
    expect(foot.querySelector(".board-confirm-key")).toHaveTextContent("↵");
  });

  // The overlay must BE the board confirm's shell, not a lookalike: same
  // backdrop, same overlay card class, same footer/key elements.
  it("wears the board confirm's chrome", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    const card = consentCard()!;
    expect(card).toHaveClass("board-confirm");
    expect(card.closest(".board-confirm-backdrop")).not.toBeNull();
    expect(card.getAttribute("role")).toBe("alertdialog");
  });
});

describe("answering", () => {
  it("↵ persists true, then dispatches", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    // Nothing has entered yet — the backend reads consent at launch time.
    expect(issueEnterColumn).not.toHaveBeenCalled();

    press("Enter");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    expect(updateProjectSettings).toHaveBeenCalledWith("p1", {
      ...SETTINGS,
      featureDossiers: true,
    });
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
  });

  // The decline is the one that fails silently if it is wrong: an omitted
  // write leaves null, which is "never asked", which asks again forever.
  it("esc persists false — and still dispatches", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());

    press("Escape");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    expect(updateProjectSettings).toHaveBeenCalledWith("p1", {
      ...SETTINGS,
      featureDossiers: false,
    });
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
    expect(consentCard()).toBeNull();
  });

  // Full-replace hazard: the write must carry the app's own bookkeeping
  // through, or answering the card forgets the seeded scaffold version.
  it("carries feature_log_seeded_version through the write", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    press("Enter");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    const [, sent] = vi.mocked(updateProjectSettings).mock.calls[0];
    expect(sent.featureLogSeededVersion).toBe(3);
    expect(sent.tmux).toBe(true);
  });
});

describe("sequencing", () => {
  // A park the board did NOT raise — a settle that chained into a
  // queue-mode step, or one rehydrated after a webview reload. It is still
  // an agent_step entry, so consent still comes first; there is just
  // nothing to dispatch afterwards, because the entry already happened
  // backend-side.
  it("comes before a park the board only reconciled", async () => {
    useSteps.setState({ byIssue: { iss_a: { queuedColumnId: "c-plan" } } });
    await renderBoard("c-plan");
    await waitFor(() => expect(consentCard()).not.toBeNull());
    expect(confirmCard()).toBeNull();

    press("Enter");
    await waitFor(() => expect(confirmCard()).not.toBeNull());
    expect(updateProjectSettings).toHaveBeenCalledWith("p1", {
      ...SETTINGS,
      featureDossiers: true,
    });
    // The card already entered its column — answering must not re-enter.
    expect(issueEnterColumn).not.toHaveBeenCalled();
  });

  it("comes before the queue confirm, which then takes over", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    // Consent first: the dispatch confirm is not on screen yet.
    expect(confirmCard()).toBeNull();

    press("Enter");
    await waitFor(() => expect(confirmCard()).not.toBeNull());
    expect(consentCard()).toBeNull();
    expect(confirmCard()!.textContent).toContain("Dispatch?");
  });
});

describe("when it must NOT fire", () => {
  it("stays away from a shelf column", async () => {
    await renderBoard("c-backlog"); // ⇧L → Ready, a shelf
    moveRight();
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-ready", 0));
    expect(consentCard()).toBeNull();
  });

  it("stays away from a human gate", async () => {
    await renderBoard("c-plan"); // ⇧L → In Review, a human gate
    moveRight();
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-review", 0));
    expect(consentCard()).toBeNull();
  });

  it("never asks again once the project consented", async () => {
    vi.mocked(getProjectSettings).mockResolvedValue({ ...SETTINGS, featureDossiers: true });
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
    expect(consentCard()).toBeNull();
    expect(updateProjectSettings).not.toHaveBeenCalled();
  });

  it("never asks again once the project DECLINED", async () => {
    vi.mocked(getProjectSettings).mockResolvedValue({ ...SETTINGS, featureDossiers: false });
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
    expect(consentCard()).toBeNull();
    expect(updateProjectSettings).not.toHaveBeenCalled();
  });
});

// The card names a file the backend is about to create, so the slug rule
// has to be the backend's rule (fartcode-core/src/dossiers.rs
// `dossier_slug` → `sanitize_name`), not an approximation.
describe("slug", () => {
  it("matches the backend slugifier", () => {
    expect(dossierSlug({ id: "iss_1", title: "Implement OAuth login" })).toBe(
      "implement-oauth-login",
    );
    expect(dossierSlug({ id: "iss_1", title: "  Fix: the __thing__ (v2)!  " })).toBe(
      "fix-the-thing-v2",
    );
  });

  it("falls back to the card id when the title sanitizes to nothing", () => {
    expect(dossierSlug({ id: "iss_1a", title: "!!! ✨" })).toBe("iss-1a");
  });

  it("prefers a dossier the card already has", () => {
    expect(
      dossierPathFor({ id: "iss_1", title: "Anything", dossierPath: "docs/features/x-2.md" }),
    ).toBe("docs/features/x-2.md");
  });
});
