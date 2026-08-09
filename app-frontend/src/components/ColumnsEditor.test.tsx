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
} from "../lib/tauri";
import type { BoardColumnDto, IssueDto } from "../lib/tauri";
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

  it("locks a seeded agent step but not an empty user step", async () => {
    await renderPane();
    // Quick: user step (seedLane null), empty → an active control.
    const quick = expand("Quick");
    expect(quick.querySelector(".fc-col-delete-reason")).toBeNull();
    expect(quick.querySelector("button.fc-col-delete")).not.toBeNull();
    // In Progress: seeded agent step, empty → locked with the reason.
    expect(expand("In Progress").querySelector(".fc-col-delete-reason")).toHaveTextContent(
      "seeded step — locked until columns become authoritative",
    );
  });

  it("shows the landing reason on an empty landing column", async () => {
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
});

describe("add column", () => {
  it("appends a shelf named New column and expands it for rename", async () => {
    vi.mocked(columnCreate).mockResolvedValue(
      column({ id: "c-new", name: "New column", position: 6 }),
    );
    vi.mocked(columnList).mockResolvedValue([
      ...COLUMNS,
      column({ id: "c-new", name: "New column", position: 6 }),
    ]);
    await renderPane();
    fireEvent.click(screen.getByText("+ add column"));
    await waitFor(() =>
      expect(columnCreate).toHaveBeenCalledWith({
        projectId: "p1",
        name: "New column",
        kind: "shelf",
      }),
    );
    // The new row arrives expanded with the rename editor focused.
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
