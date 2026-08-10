// First-dispatch dossier consent (#74, handoff v3 §8e).
//
// The load-bearing assertions, in the order they'd bite:
//   1. BOTH answers persist an explicit boolean. A decline that left
//      `featureDossiers` null would fail closed backend-side AND re-ask on
//      the next dispatch — forever.
//   2. Declining still dispatches. The card gates the repo write, never
//      the agent. Only a WITHDRAWN ask (project switched) stops the entry.
//   3. Consent comes before the queue confirm, and nothing enters until it
//      is answered AND the answer has committed.
//   4. The backdrop is inert. It is the board confirm's chrome, where
//      clicking outside is harmless; here the same gesture would decline
//      permanently and dispatch.
//   5. An unreadable settings row is not "never asked" — it must not turn
//      into a prompt on every entry.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, fireEvent, render, waitFor } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
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

import BoardView from "./board/BoardView";
import DossierConsentCard from "./DossierConsentCard";
import {
  dossierPathFor,
  dossierSlug,
  ensureDossierConsent,
  useDossierConsent,
} from "../store/dossierConsent";
import { enterColumn } from "../lib/taskPipeline";
import {
  columnList,
  getProjectSettings,
  issueEnterColumn,
  issueList,
  updateProjectSettings,
  type BoardColumnDto,
  type IssueDto,
  type ProjectSettingsDto,
} from "../lib/tauri";
import { useColumns } from "../store/columns";
import { useSteps } from "../store/steps";
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

function issue(columnId: string, over: Partial<IssueDto> = {}): IssueDto {
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
    ...over,
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

/** Base settings row — carries the app-managed bookkeeping a full-replace
 * write must not drop. */
const SETTINGS: ProjectSettingsDto = {
  tmux: true,
  featureDossiers: null,
  featureLogSeededVersion: 3,
};

/** A promise the test releases by hand — the only way to observe the
 * window while a read or a write is still in flight. */
function deferred<T>() {
  let release!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    release = r;
  });
  return { promise, release };
}

function press(key: string, shiftKey = false): void {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true }));
  });
}

/** Lets queued microtasks (the gate's read/write chain) drain. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

const consentCard = () => document.querySelector(".board-consent");
const confirmCard = () => document.querySelector(".board-confirm:not(.board-consent)");

beforeEach(() => {
  vi.clearAllMocks();
  useDossierConsent.getState().reset();
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

/** The board plus the app-level consent card, which is where it really
 * renders (App.tsx) now that the task view needs it too. */
async function renderBoard(startColumn: string, over: Partial<IssueDto> = {}) {
  vi.mocked(issueList).mockResolvedValue([issue(startColumn, over)]);
  render(
    <>
      <BoardView projectId="p1" />
      <DossierConsentCard />
    </>,
  );
  await waitFor(() => expect(document.querySelectorAll(".board-card")).toHaveLength(1));
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

  // The overlay must BE the board confirm's shell, not a lookalike.
  it("wears the board confirm's chrome", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());
    const card = consentCard()!;
    expect(card).toHaveClass("board-confirm");
    expect(card.closest(".board-confirm-backdrop")).not.toBeNull();
    expect(card.getAttribute("role")).toBe("alertdialog");
  });

  // ...and BECAUSE it wears that chrome, the gesture that is harmless
  // there must be harmless here. On the board confirm, clicking the
  // backdrop keeps the card where it is and spends nothing. Wiring it to
  // decline would let a reflexive click-away opt the repo out of dossiers
  // permanently AND launch an agent.
  it("ignores a backdrop click — it neither answers nor dispatches", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());

    fireEvent.click(document.querySelector(".board-confirm-backdrop")!);
    await settle();

    expect(updateProjectSettings).not.toHaveBeenCalled();
    expect(issueEnterColumn).not.toHaveBeenCalled();
    expect(consentCard()).not.toBeNull(); // still asking
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

  // The dispatch must wait for the write to COMMIT, not merely be issued:
  // the backend reads consent at launch, so a launch that overtook the
  // write would run without the consent the user just gave.
  it("does not dispatch until the consent write has committed", async () => {
    const write = deferred<ProjectSettingsDto>();
    vi.mocked(updateProjectSettings).mockReturnValue(write.promise);
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());

    press("Enter");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    await settle();
    expect(issueEnterColumn).not.toHaveBeenCalled();

    await act(async () => write.release(SETTINGS));
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalled());
  });
});

describe("sequencing", () => {
  // A park the board did NOT raise — a settle that chained into a
  // queue-mode step, or one rehydrated after a webview reload.
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
    expect(confirmCard()).toBeNull();

    press("Enter");
    await waitFor(() => expect(confirmCard()).not.toBeNull());
    expect(consentCard()).toBeNull();
    expect(confirmCard()!.textContent).toContain("Dispatch?");
  });

  // The blocked confirm is raised BEFORE the entry, so its ↵ runs the same
  // gated entry a drag does — consent must still come first.
  it("follows the blocked confirm rather than replacing it", async () => {
    await renderBoard("c-ready", {
      blocked: true,
      blockers: [
        { id: "iss_b", title: "#9 groundwork", lane: "backlog", columnId: "c-backlog", countsAsDone: false },
      ],
    });
    moveRight();
    // Blocked first: the card is blocked and Plan is a step.
    await waitFor(() => expect(confirmCard()).not.toBeNull());
    expect(confirmCard()!.textContent).toContain("Send to Plan anyway?");
    expect(consentCard()).toBeNull();

    press("Enter"); // confirm the blocked dispatch → now the gate runs
    await waitFor(() => expect(consentCard()).not.toBeNull());
    expect(issueEnterColumn).not.toHaveBeenCalled();

    press("Enter"); // consent
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
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

  // An unreadable row and "never asked" look alike and behave oppositely.
  // Turning a broken read into a prompt would re-ask on EVERY entry, and
  // the answer could not be stored either — an undismissable loop. Safe to
  // skip: the backend reads the same broken row and writes nothing.
  it("dispatches without asking when the settings row cannot be read", async () => {
    vi.mocked(getProjectSettings).mockRejectedValue(new Error("ipc down"));
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan", 0));
    expect(consentCard()).toBeNull();
    expect(updateProjectSettings).not.toHaveBeenCalled();

    // And it does not start asking on the next entry either.
    moveRight();
    await settle();
    expect(consentCard()).toBeNull();
  });
});

describe("while the consent read is still in flight", () => {
  it("holds the entry, then asks once the read lands", async () => {
    const read = deferred<ProjectSettingsDto>();
    vi.mocked(getProjectSettings).mockReturnValue(read.promise);
    await renderBoard("c-ready");

    moveRight();
    await settle();
    // Nothing decided yet: no card, and above all no dispatch.
    expect(consentCard()).toBeNull();
    expect(issueEnterColumn).not.toHaveBeenCalled();

    await act(async () => read.release(SETTINGS));
    await waitFor(() => expect(consentCard()).not.toBeNull());
  });

  // Two entries fired inside the synchronous window before either has
  // awaited anything must join ONE ask. A guard local to any single caller
  // could not do this — the two entries can come from different surfaces.
  it("raises exactly one card for two rapid entries", async () => {
    const read = deferred<ProjectSettingsDto>();
    vi.mocked(getProjectSettings).mockReturnValue(read.promise);
    await renderBoard("c-ready");

    moveRight();
    moveRight();
    await act(async () => read.release(SETTINGS));
    await waitFor(() => expect(consentCard()).not.toBeNull());

    expect(document.querySelectorAll(".board-consent")).toHaveLength(1);
    expect(getProjectSettings).toHaveBeenCalledTimes(1);
  });
});

// Consent belongs to the project that was ASKED. BoardView is not
// remounted on a project switch, so an ask left on screen used to write
// its answer to whatever project was mounted by the time ↵ landed.
describe("project switch", () => {
  it("withdraws the ask without writing or dispatching", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());

    // What App.tsx's effect does when the selection changes.
    act(() => useDossierConsent.getState().cancelForeignAsk("p2"));
    await settle();

    expect(consentCard()).toBeNull();
    expect(updateProjectSettings).not.toHaveBeenCalled();
    expect(issueEnterColumn).not.toHaveBeenCalled();
  });

  it("keeps an ask about the project still on screen", async () => {
    await renderBoard("c-ready");
    moveRight();
    await waitFor(() => expect(consentCard()).not.toBeNull());

    act(() => useDossierConsent.getState().cancelForeignAsk("p1"));
    await settle();
    expect(consentCard()).not.toBeNull();
  });

  // The queue confirm has the same cross-project shape as the card, and
  // BoardView is not remounted by a switch: a confirm raised for project A
  // would otherwise sit over project B's board with a live ↵.
  it("drops a queue confirm raised for the project we left", async () => {
    vi.mocked(getProjectSettings).mockResolvedValue({ ...SETTINGS, featureDossiers: true });
    vi.mocked(issueList).mockResolvedValue([issue("c-ready")]);
    const view = render(
      <>
        <BoardView projectId="p1" />
        <DossierConsentCard />
      </>,
    );
    await waitFor(() => expect(document.querySelectorAll(".board-card")).toHaveLength(1));
    fireEvent.click(document.querySelector(".board-card")!);
    moveRight();
    await waitFor(() => expect(confirmCard()).not.toBeNull());

    view.rerender(
      <>
        <BoardView projectId="p2" />
        <DossierConsentCard />
      </>,
    );
    await waitFor(() => expect(confirmCard()).toBeNull());
  });

  it("writes to the asked project, not the mounted one", async () => {
    render(<DossierConsentCard />);
    const gate = ensureDossierConsent("p-asked", issue("c-plan"));
    await waitFor(() => expect(consentCard()).not.toBeNull());
    press("Enter");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    expect(vi.mocked(updateProjectSettings).mock.calls[0][0]).toBe("p-asked");
    expect(getProjectSettings).toHaveBeenCalledWith("p-asked");
    await expect(gate).resolves.toBe(true);
  });
});

// The task view makes the same `issue_enter_column` call a board drag
// makes, so it must ask the same question.
describe("the task view's enterColumn", () => {
  it("asks before entering an agent step", async () => {
    render(<DossierConsentCard />);
    const step = COLUMNS.find((c) => c.kind === "agent_step")!;
    const run = enterColumn(issue("c-ready"), step);
    await waitFor(() => expect(consentCard()).not.toBeNull());
    expect(issueEnterColumn).not.toHaveBeenCalled();

    press("Enter");
    await expect(run).resolves.toBe("queued");
    expect(issueEnterColumn).toHaveBeenCalledWith("iss_a", "c-plan");
  });

  it("does not ask for a shelf", async () => {
    render(<DossierConsentCard />);
    await expect(enterColumn(issue("c-backlog"), COLUMNS[1])).resolves.toBe("queued");
    expect(consentCard()).toBeNull();
  });

  it("reports inert when the ask is withdrawn", async () => {
    render(<DossierConsentCard />);
    const step = COLUMNS.find((c) => c.kind === "agent_step")!;
    const run = enterColumn(issue("c-ready"), step);
    await waitFor(() => expect(consentCard()).not.toBeNull());
    act(() => useDossierConsent.getState().cancelForeignAsk("p2"));
    await expect(run).resolves.toBe("inert");
    expect(issueEnterColumn).not.toHaveBeenCalled();
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
