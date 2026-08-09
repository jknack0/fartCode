import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// The tauri bridge is imported transitively by the sidebar/ui stores and by
// useCommands. Nothing in this test should reach the backend.
vi.mock("../lib/tauri", () => ({
  createTask: vi.fn(),
  createProject: vi.fn(),
  deleteProject: vi.fn(),
  deleteTask: vi.fn(),
  listProjects: vi.fn(() => Promise.resolve([])),
  listTasks: vi.fn(() => Promise.resolve([])),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  projectGitPull: vi.fn(() => Promise.resolve()),
  setViewState: vi.fn(() => Promise.resolve()),
  getViewState: vi.fn(() => Promise.resolve(null)),
  togglePin: vi.fn(),
}));

import Nav from "./Nav";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";
import type { ProjectDto, TaskDto } from "../lib/tauri";

function project(id: string, name: string): ProjectDto {
  return {
    id,
    name,
    path: `/Users/dev/${name}`,
    workspaceProvider: "git",
    baseRef: "main",
    repositoryWorkspaceId: null,
    createdAt: null,
    updatedAt: null,
  };
}

function task(over: Partial<TaskDto> & { id: string; projectId: string }): TaskDto {
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

const ALPHA = project("p-alpha", "Alpha");
const BETA = project("p-beta", "Beta");

beforeEach(() => {
  useSidebar.setState({
    projects: [ALPHA, BETA],
    tasksByProject: {
      [ALPHA.id]: [task({ id: "t-a", projectId: ALPHA.id, name: "alpha task" })],
      [BETA.id]: [
        task({ id: "t-b", projectId: BETA.id, name: "beta task", status: "review" }),
      ],
    },
    collapsed: {},
    selectedProjectId: ALPHA.id,
    selectedTaskId: null,
  });
  useUi.setState({ sidebarVisible: true, settingsOpen: false });
});

describe("Nav rail → flyout", () => {
  it("reopens a collapsed flyout when a project tile is clicked", async () => {
    // The regression this covers: with the flyout collapsed, clicking a rail
    // tile used to select the project while leaving the flyout hidden, so ⌘\
    // was the only way back.
    useUi.setState({ sidebarVisible: false });
    render(<Nav />);

    expect(screen.queryByText("Alpha")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Beta" }));

    expect(useSidebar.getState().selectedProjectId).toBe(BETA.id);
    expect(useUi.getState().sidebarVisible).toBe(true);
    // The flyout is back, showing the newly selected project.
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.getByText("beta task")).toBeInTheDocument();
  });

  it("switches the flyout to the clicked project when already visible", async () => {
    render(<Nav />);
    expect(screen.getByText("alpha task")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Beta" }));

    expect(useSidebar.getState().selectedProjectId).toBe(BETA.id);
    expect(screen.getByText("beta task")).toBeInTheDocument();
    expect(screen.queryByText("alpha task")).not.toBeInTheDocument();
  });

  it("marks the selected tile active", async () => {
    render(<Nav />);
    expect(screen.getByRole("button", { name: "Alpha" }).className).toContain("active");
    expect(screen.getByRole("button", { name: "Beta" }).className).not.toContain("active");

    await userEvent.click(screen.getByRole("button", { name: "Beta" }));

    expect(screen.getByRole("button", { name: "Beta" }).className).toContain("active");
    expect(screen.getByRole("button", { name: "Alpha" }).className).not.toContain("active");
  });

  it("collapses the flyout from its own control", async () => {
    render(<Nav />);
    await userEvent.click(screen.getByRole("button", { name: "Collapse project flyout" }));

    expect(useUi.getState().sidebarVisible).toBe(false);
    expect(screen.queryByText("alpha task")).not.toBeInTheDocument();
  });

  it("groups in-flight work ahead of Recent", () => {
    useSidebar.setState({
      selectedProjectId: BETA.id,
      tasksByProject: {
        ...useSidebar.getState().tasksByProject,
        [BETA.id]: [
          task({ id: "t-done", projectId: BETA.id, name: "shipped", status: "done" }),
          task({ id: "t-run", projectId: BETA.id, name: "running now", status: "in_progress" }),
          task({ id: "t-rev", projectId: BETA.id, name: "waiting on you", status: "review" }),
        ],
      },
    });
    render(<Nav />);

    const labels = screen
      .getAllByText(/^(Needs you|Running|Recent)$/)
      .map((el) => el.textContent);
    expect(labels).toEqual(["Needs you", "Running", "Recent"]);
  });

  it("hides archived tasks from the flyout", () => {
    useSidebar.setState({
      tasksByProject: {
        ...useSidebar.getState().tasksByProject,
        [ALPHA.id]: [
          task({ id: "t-a", projectId: ALPHA.id, name: "alpha task" }),
          task({
            id: "t-old",
            projectId: ALPHA.id,
            name: "archived task",
            archivedAt: "2026-01-01T00:00:00Z",
          }),
        ],
      },
    });
    render(<Nav />);

    expect(screen.getByText("alpha task")).toBeInTheDocument();
    expect(screen.queryByText("archived task")).not.toBeInTheDocument();
  });
});
