// #135: deleting a task silently strands its board card — the FK clears
// `linked_task_id` with no confirm mention. The delete confirm is the one
// itemizing surface (§7a), so it must list the card it will unlink.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../lib/useCommands", () => ({ hint: () => "" }));
vi.mock("../lib/tauri", () => ({
  terminalListForTask: vi.fn(() => Promise.resolve([])),
  listLineComments: vi.fn(() => Promise.resolve([])),
  gitCommitState: vi.fn(() => Promise.resolve({ branch: null })),
  issueList: vi.fn(() => Promise.resolve([])),
  archiveTask: vi.fn(),
}));

import Modals from "./Modals";
import { issueList, type IssueDto, type TaskDto } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

const TASK: TaskDto = {
  id: "t1",
  projectId: "p1",
  name: "demo",
  status: "in_progress",
  linkedIssue: null,
  archivedAt: null,
  isPinned: false,
  lastInteractedAt: null,
  statusChangedAt: null,
  workspaceId: null,
  createdBy: "user",
  type: "task",
};

function card(over: Partial<IssueDto>): IssueDto {
  return {
    id: "i1",
    projectId: "p1",
    title: "Reconnect buffer queue",
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
    columnId: null,
    blocked: false,
    blockers: [],
    createdAt: null,
    updatedAt: null,
    ...over,
  };
}

beforeEach(() => {
  useSidebar.setState({ tasksByProject: { p1: [TASK] }, projects: [] });
  useUi.setState({ deleteTaskTarget: { projectId: "p1", taskId: "t1" } });
});

describe("DeleteTaskConfirm unlinks row (#135)", () => {
  it('lists unlinks card "<title>" when a board card links the task', async () => {
    vi.mocked(issueList).mockResolvedValue([
      card({ linkedTaskId: "t1" }),
      // Unlinked sibling — must not appear in the confirm.
      card({ id: "i2", title: "Other card" }),
      // Second linked card (adversarial finding 3): one row per card.
      card({ id: "i3", title: "Second linked", linkedTaskId: "t1" }),
    ]);

    render(<Modals />);

    await waitFor(() =>
      expect(screen.getByText('unlinks card "Reconnect buffer queue"')).toBeTruthy(),
    );
    expect(screen.getByText('unlinks card "Second linked"')).toBeTruthy();
    expect(screen.queryByText(/Other card/)).toBeNull();
  });

  it("middle-ellipsizes a long card title like the confirm title does", async () => {
    // 50 chars → truncate(…, 36): 21 head + "…" + 14 tail.
    const long = "A".repeat(25) + "B".repeat(25);
    vi.mocked(issueList).mockResolvedValue([
      card({ linkedTaskId: "t1", title: long }),
    ]);

    render(<Modals />);

    const want = `unlinks card "${"A".repeat(21)}…${"B".repeat(14)}"`;
    await waitFor(() => expect(screen.getByText(want)).toBeTruthy());
    expect(screen.queryByText(new RegExp(long))).toBeNull();
  });

  it("omits the unlinks row when no card links the task", async () => {
    vi.mocked(issueList).mockResolvedValue([card({ id: "i2" })]);

    render(<Modals />);

    await waitFor(() => expect(vi.mocked(issueList)).toHaveBeenCalledWith("p1"));
    // Adversarial finding 6: flush the resolved fetch before asserting
    // absence — otherwise a late-rendered row could slip past.
    await act(async () => {});
    expect(screen.queryByText(/unlinks card/)).toBeNull();
  });
});
