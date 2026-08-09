// Columns editor pane (#67, handoff v3 §8d).
//
// The load-bearing assertions: the collapsed summary is BYTE-identical to
// columnConfigSummary's output (one formatter — the board header and this
// pane must never drift apart), delete is a disabled label with the exact
// client-side reason (never a dialog), and the tri-state patch discipline
// holds — clearing the advance target sends an EXPLICIT null, because the
// backend reads an absent key as "keep".

import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  columnList: vi.fn(() => Promise.resolve([])),
  columnCreate: vi.fn(),
  columnUpdate: vi.fn(),
  columnDelete: vi.fn(() => Promise.resolve()),
  columnReorder: vi.fn(() => Promise.resolve([])),
  issueList: vi.fn(() => Promise.resolve([])),
  hostDependencyList: vi.fn(() => Promise.resolve([])),
  hostDependencyRegistrySummary: vi.fn(() => Promise.resolve(null)),
  hostDependencyInstall: vi.fn(),
  hostDependencyUpdate: vi.fn(),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));

import { ColumnsPane } from "./ColumnsEditor";
import {
  columnCreate,
  columnDelete,
  columnList,
  columnReorder,
  columnUpdate,
  issueList,
  onFartcodeEvent,
} from "../lib/tauri";
import type { BoardColumnDto, FartcodeEvent, IssueDto } from "../lib/tauri";
import { columnConfigSummary, columnSublineTone } from "../lib/columnConfig";
import { useColumns } from "../store/columns";
import { useDependencies } from "../store/dependencies";

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

function issue(id: string, columnId: string): IssueDto {
  return {
    id,
    projectId: "p1",
    title: id,
    body: null,
    acceptance: [],
    lane: "backlog",
    position: 0,
    provider: null,
    model: null,
    prdPath: null,
    prdSection: null,
    linkedTaskId: null,
    externalRef: null,
    columnId,
    blocked: false,
    blockers: [],
    createdAt: null,
    updatedAt: null,
  };
}

// The seeded template, trimmed: a landing shelf holding two cards, an
// empty shelf, a seeded agent step (queue, advance→next), a run-mode Quick
// with an explicit target, a terminal shelf, and one empty user column.
const COLUMNS = [
  column({ id: "c-backlog", name: "Backlog", position: 0, isLanding: true }),
  column({ id: "c-ready", name: "Ready", position: 1 }),
  column({
    id: "c-quick",
    name: "Quick",
    position: 2,
    kind: "agent_step",
    onEnter: "run",
    onSettle: "advance",
    advanceTo: "c-done",
    stepProvider: "claude",
    stepModel: "haiku",
  }),
  column({
    id: "c-progress",
    name: "In Progress",
    position: 3,
    kind: "agent_step",
    onEnter: "queue",
    onSettle: "advance",
    seedLane: "in_progress",
  }),
  column({ id: "c-done", name: "Done", position: 4, countsAsDone: true }),
  column({ id: "c-user", name: "Later", position: 5 }),
];

const ISSUES = [issue("a", "c-backlog"), issue("b", "c-backlog")];

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(columnList).mockResolvedValue(COLUMNS);
  vi.mocked(issueList).mockResolvedValue(ISSUES);
  vi.mocked(columnUpdate).mockResolvedValue(COLUMNS[0]);
  useColumns.setState({ byProject: {}, loading: {}, loaded: {}, error: null });
  useDependencies.setState({
    deps: [],
    summary: null,
    loading: false,
    error: null,
    installing: {},
  });
});

async function renderPane() {
  const view = render(<ColumnsPane projectId="p1" />);
  await waitFor(() => expect(screen.getByText("Backlog")).toBeInTheDocument());
  // Occupancy must have landed before delete-reason assertions make sense.
  await waitFor(() => expect(issueList).toHaveBeenCalled());
  return view;
}

function rowFor(name: string): HTMLElement {
  const el = screen
    .getByText(name, { selector: ".fc-col-name" })
    .closest(".fc-col-row-wrap") as HTMLElement | null;
  if (!el) throw new Error(`no row for ${name}`);
  return el;
}

function expand(name: string): HTMLElement {
  const row = rowFor(name);
  fireEvent.click(row.querySelector(".fc-col-row")!);
  return row;
}

describe("collapsed rows", () => {
  it("renders THE columnConfigSummary string, byte-identical, with its tone", async () => {
    await renderPane();
    for (const c of COLUMNS) {
      const summary = rowFor(c.name).querySelector(".fc-col-summary")!;
      expect(summary.textContent).toBe(
        columnConfigSummary(c, { columns: COLUMNS, defaultAgent: "claude" }),
      );
      expect(summary.getAttribute("data-tone")).toBe(columnSublineTone(c));
    }
    // Spot-check the strings themselves so a formatter regression is loud.
    expect(rowFor("Quick").querySelector(".fc-col-summary")!.textContent).toBe(
      "claude · haiku — run → Done",
    );
    expect(rowFor("In Progress").querySelector(".fc-col-summary")!.textContent).toBe(
      "claude — queue → Done",
    );
    expect(rowFor("Done").querySelector(".fc-col-summary")!.textContent).toBe(
      "counts as done",
    );
  });

  it("tags the landing column with a landing tag", async () => {
    await renderPane();
    expect(rowFor("Backlog").querySelector(".fc-col-landing")).toHaveTextContent("landing");
    expect(rowFor("Ready").querySelector(".fc-col-landing")).toBeNull();
  });
});

describe("delete", () => {
  it("is a disabled label with the occupancy reason on an occupied column", async () => {
    await renderPane();
    const row = expand("Backlog");
    expect(row.querySelector(".fc-col-delete-disabled")).toHaveTextContent("delete column");
    // The occupancy fetch is async — the reason settles once issues land.
    await waitFor(() =>
      expect(row.querySelector(".fc-col-delete-reason")).toHaveTextContent(
        "2 cards live here — move them first",
      ),
    );
    expect(row.querySelector("button.fc-col-delete")).toBeNull();
  });

  it("no longer locks empty agent steps — seeded or not (E18-07)", async () => {
    await renderPane();
    // Quick: user step, empty, not a target → an active control.
    const quick = expand("Quick");
    expect(quick.querySelector(".fc-col-delete-reason")).toBeNull();
    expect(quick.querySelector("button.fc-col-delete")).not.toBeNull();
    // In Progress: SEEDED agent step, empty — the pre-flip lock is gone.
    const progress = expand("In Progress");
    expect(progress.querySelector(".fc-col-delete-reason")).toBeNull();
    expect(progress.querySelector("button.fc-col-delete")).not.toBeNull();
  });

  it("shows the advance-target reason on a column another column advances to", async () => {
    await renderPane();
    // Done is Quick's advanceTo target → disabled with the typed reason.
    const done = expand("Done");
    expect(done.querySelector(".fc-col-delete-reason")).toHaveTextContent(
      "advance target of Quick — repoint it first",
    );
    expect(done.querySelector("button.fc-col-delete")).toBeNull();
  });

  it("occupied wins over the advance-target reason (spec priority)", async () => {
    vi.mocked(issueList).mockResolvedValue([issue("d1", "c-done")]);
    await renderPane();
    const done = expand("Done");
    await waitFor(() =>
      expect(done.querySelector(".fc-col-delete-reason")).toHaveTextContent(
        "1 card lives here — move it first",
      ),
    );
  });

  it("shows the landing reason on an empty landing column", async () => {
    vi.mocked(issueList).mockResolvedValue([]);
    await renderPane();
    expect(expand("Backlog").querySelector(".fc-col-delete-reason")).toHaveTextContent(
      "landing column — move landing first",
    );
  });

  it("landing wins over the advance-target reason (spec priority)", async () => {
    // Backlog is BOTH the landing column and Quick's advanceTo target;
    // with no issues anywhere, the landing reason must win.
    vi.mocked(columnList).mockResolvedValue(
      COLUMNS.map((c) => (c.id === "c-quick" ? { ...c, advanceTo: "c-backlog" } : c)),
    );
    vi.mocked(issueList).mockResolvedValue([]);
    await renderPane();
    expect(expand("Backlog").querySelector(".fc-col-delete-reason")).toHaveTextContent(
      "landing column — move landing first",
    );
  });

  it("is an active control on an empty user column and calls columnDelete", async () => {
    await renderPane();
    const row = expand("Later");
    const del = row.querySelector("button.fc-col-delete")!;
    expect(row.querySelector(".fc-col-delete-reason")).toBeNull();
    fireEvent.click(del);
    await waitFor(() => expect(columnDelete).toHaveBeenCalledWith("c-user"));
    // Every successful mutation reloads the store for the open board.
    await waitFor(() => expect(columnList).toHaveBeenCalledTimes(2));
  });

  it("uses the SINGULAR reason when exactly one card lives here", async () => {
    vi.mocked(issueList).mockResolvedValue([issue("solo", "c-user")]);
    await renderPane();
    const row = expand("Later");
    await waitFor(() =>
      expect(row.querySelector(".fc-col-delete-reason")).toHaveTextContent(
        "1 card lives here — move it first",
      ),
    );
    expect(row.querySelector("button.fc-col-delete")).toBeNull();
  });

  it("re-checks occupancy on FRESH data and aborts when a card just arrived", async () => {
    await renderPane();
    const row = expand("Later");
    // The step engine settles a card into Later after the button rendered —
    // the pre-delete refetch must catch it and never call the backend.
    vi.mocked(issueList).mockResolvedValue([issue("sneaky", "c-user")]);
    fireEvent.click(row.querySelector("button.fc-col-delete")!);
    await waitFor(() =>
      expect(row.querySelector(".fc-col-delete-reason")).toHaveTextContent(
        "1 card lives here — move it first",
      ),
    );
    expect(columnDelete).not.toHaveBeenCalled();
  });
});

describe("add column", () => {
  it("appends a shelf named New column and expands it for rename", async () => {
    vi.mocked(columnCreate).mockResolvedValue(
      column({ id: "c-new", name: "New column", position: 6 }),
    );
    // Sequenced on purpose: the initial load must NOT contain the new row —
    // it only exists after the post-create reload, so a pane that skipped
    // that reload fails here.
    vi.mocked(columnList)
      .mockResolvedValueOnce(COLUMNS)
      .mockResolvedValue([...COLUMNS, column({ id: "c-new", name: "New column", position: 6 })]);
    await renderPane();
    expect(screen.queryByText("New column", { selector: ".fc-col-name" })).toBeNull();
    fireEvent.click(screen.getByText("+ add column"));
    await waitFor(() =>
      expect(columnCreate).toHaveBeenCalledWith({
        projectId: "p1",
        name: "New column",
        kind: "shelf",
      }),
    );
    // The reload happened, and the new row arrives expanded with the rename
    // editor focused.
    await waitFor(() => expect(columnList).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(rowFor("New column").className).toContain("open"));
    const input = rowFor("New column").querySelector<HTMLInputElement>(".fc-set-editor input")!;
    expect(input.value).toBe("New column");
  });
});

describe("field patches", () => {
  it("an on-enter change sends {onEnter} and nothing else", async () => {
    await renderPane();
    expand("In Progress");
    fireEvent.click(screen.getByText("on enter").closest("button")!);
    fireEvent.click(screen.getByRole("button", { name: "run" }));
    await waitFor(() =>
      expect(columnUpdate).toHaveBeenCalledWith("c-progress", { onEnter: "run" }),
    );
    const calls = vi.mocked(columnUpdate).mock.calls;
    const patch = calls[calls.length - 1][1];
    expect(Object.keys(patch)).toEqual(["onEnter"]);
  });

  it("clearing the advance target sends an EXPLICIT {advanceTo: null}", async () => {
    await renderPane();
    expand("Quick");
    fireEvent.click(screen.getByText("on settle").closest("button")!);
    fireEvent.click(screen.getByRole("button", { name: "next column" }));
    await waitFor(() =>
      expect(columnUpdate).toHaveBeenCalledWith("c-quick", { advanceTo: null }),
    );
    const calls = vi.mocked(columnUpdate).mock.calls;
    const patch = calls[calls.length - 1][1];
    expect(Object.keys(patch)).toEqual(["advanceTo"]);
    expect(patch.advanceTo).toBeNull();
  });

  it("splits tools on newlines only — a space-containing entry survives", async () => {
    await renderPane();
    const row = expand("Quick");
    fireEvent.click(within(row).getByText("tools").closest("button")!);
    const ta = row.querySelector("textarea")!;
    fireEvent.change(ta, { target: { value: "Read\nwrite plan.md\n\n Bash \n" } });
    fireEvent.blur(ta);
    await waitFor(() =>
      expect(columnUpdate).toHaveBeenCalledWith("c-quick", {
        stepTools: ["Read", "write plan.md", "Bash"],
      }),
    );
  });

  it("renders [] as none and never patches an unchanged tools editor away", async () => {
    // stepTools [] is a real wire state (empty allowlist; corrupt cells
    // parse fail-closed as []). Open + blur must NOT flip it to null.
    vi.mocked(columnList).mockResolvedValue(
      COLUMNS.map((c) => (c.id === "c-quick" ? { ...c, stepTools: [] as string[] } : c)),
    );
    await renderPane();
    const row = expand("Quick");
    const toolsRow = within(row).getByText("tools").closest("button")!;
    expect(toolsRow.querySelector(".fc-set-value")).toHaveTextContent("none");
    fireEvent.click(toolsRow);
    fireEvent.blur(row.querySelector("textarea")!);
    // The editor closed without a write.
    await waitFor(() => expect(row.querySelector("textarea")).toBeNull());
    expect(columnUpdate).not.toHaveBeenCalled();
  });

  it("does not patch when the prompt editor closes unchanged", async () => {
    await renderPane();
    const row = expand("Ready");
    fireEvent.click(row.querySelector(".fc-col-prompt")!);
    fireEvent.blur(row.querySelector(".fc-col-prompt-wrap textarea")!);
    await waitFor(() => expect(row.querySelector(".fc-col-prompt-wrap textarea")).toBeNull());
    expect(columnUpdate).not.toHaveBeenCalled();
  });

  it("ignores a second counts-as-done click while the first is in flight", async () => {
    let release!: (v: BoardColumnDto) => void;
    vi.mocked(columnUpdate).mockImplementation(
      () =>
        new Promise<BoardColumnDto>((res) => {
          release = res;
        }),
    );
    await renderPane();
    const row = expand("Ready");
    const btn = within(row).getByText("counts as done").closest("button")!;
    fireEvent.click(btn);
    // Rows advertise the in-flight window; the second click computes from
    // the stale store and must be ignored, not sent.
    expect(rowFor("Ready").getAttribute("aria-busy")).toBe("true");
    fireEvent.click(btn);
    expect(columnUpdate).toHaveBeenCalledTimes(1);
    release(column({ id: "c-ready", name: "Ready", position: 1, countsAsDone: true }));
    await waitFor(() => expect(rowFor("Ready").getAttribute("aria-busy")).toBe("false"));
  });
});

describe("reorder", () => {
  it("drops the dragged column and sends the COMPLETE id list", async () => {
    await renderPane();
    const handle = rowFor("Ready").querySelector(".fc-col-handle")!;
    fireEvent.dragStart(handle, {
      dataTransfer: { setData: () => {}, effectAllowed: "" },
    });
    const target = rowFor("Done");
    fireEvent.dragOver(target);
    fireEvent.drop(target);
    // jsdom rects are 0-height, so the drop lands AFTER the target row.
    await waitFor(() =>
      expect(columnReorder).toHaveBeenCalledWith("p1", [
        "c-backlog",
        "c-quick",
        "c-progress",
        "c-done",
        "c-ready",
        "c-user",
      ]),
    );
  });

  it("tracks the pointer half: indicator edge and insert side agree", async () => {
    await renderPane();
    // The header row is what onDragOver measures — give it a real rect
    // (top 100, height 40 → midpoint 120).
    const mockRect = (wrap: HTMLElement) => {
      (wrap.querySelector(".fc-col-row") as HTMLElement).getBoundingClientRect = () =>
        ({
          top: 100,
          bottom: 140,
          height: 40,
          left: 0,
          right: 200,
          width: 200,
          x: 0,
          y: 100,
          toJSON: () => ({}),
        }) as DOMRect;
    };
    const startDrag = () =>
      fireEvent.dragStart(rowFor("Ready").querySelector(".fc-col-handle")!, {
        dataTransfer: { setData: () => {}, effectAllowed: "" },
      });
    // jsdom has no DragEvent, and testing-library's plain-Event fallback
    // drops clientY — dispatch MouseEvents of the drag type instead.
    const dragOverAt = (el: HTMLElement, clientY: number) =>
      fireEvent(el, new MouseEvent("dragover", { bubbles: true, cancelable: true, clientY }));
    const dropAt = (el: HTMLElement, clientY: number) =>
      fireEvent(el, new MouseEvent("drop", { bubbles: true, cancelable: true, clientY }));

    // Upper half (105 < 120): the accent line sits on the TOP edge and the
    // dragged row inserts BEFORE the target.
    startDrag();
    let wrap = rowFor("Done");
    mockRect(wrap);
    dragOverAt(wrap, 105);
    expect(wrap.className).toContain("drop-before");
    dropAt(wrap, 105);
    await waitFor(() =>
      expect(columnReorder).toHaveBeenLastCalledWith("p1", [
        "c-backlog",
        "c-quick",
        "c-progress",
        "c-ready",
        "c-done",
        "c-user",
      ]),
    );
    // Wait out the in-flight window (drags are suppressed while busy).
    await waitFor(() => expect(rowFor("Done").getAttribute("aria-busy")).toBe("false"));

    // Lower half (135 > 120): BOTTOM edge, inserts AFTER.
    startDrag();
    wrap = rowFor("Done");
    mockRect(wrap);
    dragOverAt(wrap, 135);
    expect(wrap.className).toContain("drop-after");
    expect(wrap.className).not.toContain("drop-before");
    dropAt(wrap, 135);
    await waitFor(() =>
      expect(columnReorder).toHaveBeenLastCalledWith("p1", [
        "c-backlog",
        "c-quick",
        "c-progress",
        "c-done",
        "c-ready",
        "c-user",
      ]),
    );
  });
});

describe("occupancy", () => {
  it("refetches when the step engine or issue CRUD fires for this project", async () => {
    let emit: ((ev: FartcodeEvent) => void) | null = null;
    vi.mocked(onFartcodeEvent).mockImplementation((cb) => {
      emit = cb;
      return Promise.resolve(() => {});
    });
    await renderPane();
    const calls = vi.mocked(issueList).mock.calls.length;
    emit!({
      type: "step:settled",
      issueId: "a",
      projectId: "p1",
      columnId: "c-done",
      taskId: "t1",
    });
    await waitFor(() => expect(issueList).toHaveBeenCalledTimes(calls + 1));
    // Another project's churn does not refetch this pane.
    emit!({ type: "issue:updated", id: "x", projectId: "other" });
    await new Promise((r) => setTimeout(r, 0));
    expect(issueList).toHaveBeenCalledTimes(calls + 1);
  });
});

describe("errors", () => {
  it("surfaces a backend rejection in the pane's error line", async () => {
    vi.mocked(columnUpdate).mockRejectedValue("landing column cannot unset");
    await renderPane();
    const row = expand("Ready");
    // Toggle counts-as-done — a one-click patch path. ("counts as done" is
    // also Done's summary string, so scope the query to the expanded row.)
    fireEvent.click(within(row).getByText("counts as done").closest("button")!);
    await waitFor(() =>
      expect(document.querySelector(".fc-set-error")).toHaveTextContent(
        "landing column cannot unset",
      ),
    );
  });
});
