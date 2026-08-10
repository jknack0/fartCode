// Settings nav children (#76): Memory joins Columns under the expanded
// project row, section strings parse generically ("project:<id>[:<child>]"),
// and each child routes to its own pane with a "<Child> · <project>" title.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  telemetryMemoryValue: vi.fn(() => new Promise(() => {})),
  getViewState: vi.fn(() => Promise.resolve(null)),
  setViewState: vi.fn(() => Promise.resolve()),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock("../lib/useCommands", () => ({
  bindings: () => [],
  clearAllOverrides: vi.fn(() => Promise.resolve()),
  hint: () => "",
  saveOverride: vi.fn(() => Promise.resolve()),
}));
vi.mock("./AgentsList", () => ({ default: () => null }));
vi.mock("./ProviderAccounts", () => ({ default: () => null }));
vi.mock("./ColumnsEditor", () => ({
  ColumnsPane: ({ projectId }: { projectId: string }) => (
    <div data-testid="columns-pane">{projectId}</div>
  ),
}));
vi.mock("./ProjectSettings", () => ({
  ProjectSettingsPane: ({ projectId }: { projectId: string }) => (
    <div data-testid="project-pane">{projectId}</div>
  ),
}));

import SettingsModal from "./SettingsModal";
import { useSidebar } from "../store/sidebar";
import type { ProjectDto } from "../lib/tauri";

const PROJECT = {
  id: "p1",
  name: "fartCode",
  path: "/tmp/fartcode",
} as unknown as ProjectDto;

beforeEach(() => {
  vi.clearAllMocks();
  useSidebar.setState({ projects: [PROJECT] });
});

describe("SettingsModal project children", () => {
  it("shows Memory beside Columns under the expanded project row", () => {
    render(<SettingsModal onClose={() => {}} initialSection="project:p1" />);
    expect(screen.getByRole("button", { name: "Columns" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Memory" })).toBeTruthy();
    // The project pane is the default child-less view.
    expect(screen.getByTestId("project-pane").textContent).toBe("p1");
  });

  it("routes the Memory child to the memory pane and titles it", () => {
    render(<SettingsModal onClose={() => {}} initialSection="project:p1" />);
    fireEvent.click(screen.getByRole("button", { name: "Memory" }));
    // Pane fetch is pending forever in this mock — the loading state IS the pane.
    expect(screen.getByText("loading…")).toBeTruthy();
    expect(screen.getByText("Memory · fartCode")).toBeTruthy();
    expect(screen.queryByTestId("columns-pane")).toBeNull();
    expect(screen.queryByTestId("project-pane")).toBeNull();
  });

  it("still routes the Columns child to the columns pane", () => {
    render(<SettingsModal onClose={() => {}} initialSection="project:p1:columns" />);
    expect(screen.getByTestId("columns-pane").textContent).toBe("p1");
    expect(screen.getByText("Columns · fartCode")).toBeTruthy();
    expect(screen.queryByTestId("project-pane")).toBeNull();
  });

  it("opens directly onto the memory section string", () => {
    render(<SettingsModal onClose={() => {}} initialSection="project:p1:memory" />);
    expect(screen.getByText("loading…")).toBeTruthy();
    expect(screen.getByText("Memory · fartCode")).toBeTruthy();
  });
});
