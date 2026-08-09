import { describe, it, expect, vi } from "vitest";

// sidebar.ts imports the tauri bridge at module scope; nothing here calls it,
// but the real module reaches for @tauri-apps/api on import.
vi.mock("../lib/tauri", () => ({
  createTask: vi.fn(),
  createProject: vi.fn(),
  deleteProject: vi.fn(),
  deleteTask: vi.fn(),
  listProjects: vi.fn(),
  listTasks: vi.fn(),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  projectGitPull: vi.fn(() => Promise.resolve()),
  setViewState: vi.fn(() => Promise.resolve()),
  getViewState: vi.fn(() => Promise.resolve(null)),
  togglePin: vi.fn(),
}));

// vi.mock is hoisted above this import by vitest's transform.
import { visibleTaskOrder } from "./sidebar";

// SidebarState isn't exported; derive the argument/result types from the
// function so these stay honest if its signature changes.
type State = Parameters<typeof visibleTaskOrder>[0];
type Task = ReturnType<typeof visibleTaskOrder>[number];

function task(over: Partial<Task> & { id: string; projectId: string }): Task {
  return {
    name: over.id,
    status: "todo",
    linkedIssue: null,
    archivedAt: null,
    isPinned: false,
    lastInteractedAt: null,
    statusChangedAt: null,
    workspaceId: null,
    createdBy: "user",
    type: "task",
    ...over,
  };
}

/** Builds the slice of sidebar state that visibleTaskOrder actually reads. */
function state(
  projectIds: string[],
  tasksByProject: Record<string, Task[]>,
  collapsed: Record<string, boolean> = {},
): State {
  return {
    projects: projectIds.map((id) => ({
      id,
      name: id,
      path: `/tmp/${id}`,
      workspaceProvider: "git",
      baseRef: null,
      repositoryWorkspaceId: null,
      createdAt: null,
      updatedAt: null,
    })),
    tasksByProject,
    collapsed,
  } as State;
}

const ids = (tasks: Task[]) => tasks.map((t) => t.id);

describe("visibleTaskOrder", () => {
  it("puts every pinned task before every unpinned one, across projects", () => {
    const s = state(["a", "b"], {
      a: [task({ id: "a1", projectId: "a" }), task({ id: "a2", projectId: "a", isPinned: true })],
      b: [task({ id: "b1", projectId: "b", isPinned: true }), task({ id: "b2", projectId: "b" })],
    });
    // Pinned section in project tree order (a's pin, then b's), then the tree.
    expect(ids(visibleTaskOrder(s))).toEqual(["a2", "b1", "a1", "b2"]);
  });

  it("keeps project tree order and within-project task order", () => {
    const s = state(["p2", "p1"], {
      p1: [task({ id: "x", projectId: "p1" }), task({ id: "y", projectId: "p1" })],
      p2: [task({ id: "m", projectId: "p2" }), task({ id: "n", projectId: "p2" })],
    });
    // `projects` order is authoritative — p2 is listed first.
    expect(ids(visibleTaskOrder(s))).toEqual(["m", "n", "x", "y"]);
  });

  it("skips the tasks of a collapsed project", () => {
    const s = state(
      ["a", "b"],
      {
        a: [task({ id: "a1", projectId: "a" })],
        b: [task({ id: "b1", projectId: "b" })],
      },
      { a: true },
    );
    expect(ids(visibleTaskOrder(s))).toEqual(["b1"]);
  });

  it("still surfaces pinned tasks of a collapsed project", () => {
    // The pinned section is a flat list above the tree — collapsing a project
    // hides its subtree, not its pins.
    const s = state(
      ["a", "b"],
      {
        a: [
          task({ id: "a1", projectId: "a", isPinned: true }),
          task({ id: "a2", projectId: "a" }),
        ],
        b: [task({ id: "b1", projectId: "b" })],
      },
      { a: true },
    );
    expect(ids(visibleTaskOrder(s))).toEqual(["a1", "b1"]);
  });

  it("treats collapsed:false the same as absent", () => {
    const s = state(["a"], { a: [task({ id: "a1", projectId: "a" })] }, { a: false });
    expect(ids(visibleTaskOrder(s))).toEqual(["a1"]);
  });

  it("skips archived tasks in the tree", () => {
    const s = state(["a"], {
      a: [
        task({ id: "live", projectId: "a" }),
        task({ id: "gone", projectId: "a", archivedAt: "2026-01-01T00:00:00Z" }),
      ],
    });
    expect(ids(visibleTaskOrder(s))).toEqual(["live"]);
  });

  it("skips archived tasks even when pinned", () => {
    // Archive wins over pin — an archived pin must not resurface at the top.
    const s = state(["a"], {
      a: [
        task({
          id: "pinned-archived",
          projectId: "a",
          isPinned: true,
          archivedAt: "2026-01-01T00:00:00Z",
        }),
        task({ id: "live", projectId: "a" }),
      ],
    });
    expect(ids(visibleTaskOrder(s))).toEqual(["live"]);
  });

  it("never lists a task twice", () => {
    const s = state(["a"], {
      a: [task({ id: "p", projectId: "a", isPinned: true }), task({ id: "u", projectId: "a" })],
    });
    const order = ids(visibleTaskOrder(s));
    expect(order).toEqual(["p", "u"]);
    expect(new Set(order).size).toBe(order.length);
  });

  it("tolerates a project with no loaded tasks", () => {
    const s = state(["a", "b"], { b: [task({ id: "b1", projectId: "b" })] });
    expect(ids(visibleTaskOrder(s))).toEqual(["b1"]);
  });

  it("returns an empty list when there are no projects", () => {
    expect(visibleTaskOrder(state([], {}))).toEqual([]);
  });
});
