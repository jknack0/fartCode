// ⌘K feature hits (#75, handoff v3 §8h).
//
// #72 wrote one `feature` row per dossier section but the command filtered
// them out of the palette; #75 deletes that filter, so the first thing this
// suite proves is that a feature row APPEARS at all. Then the §8h shape:
// `<Column> — <feature title>` on the left, mono `feature · #id[ · landed]`
// on the right, and ↵ opening the CARD DETAIL — one destination whether the
// feature is live or landed.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../lib/useCommands", () => ({ bindings: () => [] }));

vi.mock("../lib/tauri", () => ({
  search: vi.fn(() => Promise.resolve([])),
  dossierFeatureRows: vi.fn(() => Promise.resolve([])),
  getResourceMonitorEnabled: vi.fn(() => Promise.resolve(false)),
  setResourceMonitorEnabled: vi.fn(() => Promise.resolve()),
  restoreTask: vi.fn(() => Promise.resolve()),
  // selectProject persists view state and auto-pulls the project.
  setViewState: vi.fn(() => Promise.resolve()),
  listTasks: vi.fn(() => Promise.resolve([])),
  projectGitPull: vi.fn(() => Promise.resolve()),
}));

import CommandPalette from "./CommandPalette";
import { dossierFeatureRows, search } from "../lib/tauri";
import type { FeatureRowDto, SearchResultDto } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

const FEATURE_HIT: SearchResultDto = {
  itemType: "feature",
  itemId: "i392#Plan — 2026-08-07",
  projectId: "p1",
  taskId: null,
  title: "Plan — 2026-08-07",
};

function row(over: Partial<FeatureRowDto> = {}): FeatureRowDto {
  return {
    itemId: FEATURE_HIT.itemId,
    issueId: "i392",
    title: "#392 invite vetting",
    externalRef: null,
    landed: false,
    ...over,
  };
}

/** Opens the palette and types a query past the 150ms debounce. */
async function searchFor(q: string) {
  render(<CommandPalette />);
  fireEvent.change(screen.getByLabelText("Command palette"), { target: { value: q } });
  await waitFor(() => expect(vi.mocked(search)).toHaveBeenCalled());
}

beforeEach(() => {
  vi.clearAllMocks();
  useUi.setState({ paletteOpen: true, boardDetailIssueId: null, changesOpen: false });
  useSidebar.setState({ selectedProjectId: null, selectedTaskId: null });
  vi.mocked(search).mockResolvedValue([FEATURE_HIT]);
  vi.mocked(dossierFeatureRows).mockResolvedValue([row()]);
});

describe("feature hits", () => {
  it("surfaces feature rows now that the palette filter is gone", async () => {
    await searchFor("vetting");
    // The row is here at all — #72's PALETTE_HIDDEN_TYPES used to drop it.
    await waitFor(() => expect(screen.getByText("Plan — invite vetting")).toBeTruthy());
    expect(vi.mocked(dossierFeatureRows)).toHaveBeenCalledWith([FEATURE_HIT.itemId]);
  });

  it("renders §8h's title and right meta", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([
      row({ externalRef: "https://github.com/o/r/issues/392" }),
    ]);
    await searchFor("vetting");

    // Title: the indexed heading's COLUMN half + the feature's own title.
    const title = await screen.findByText("Plan — invite vetting");
    expect(title.className).toContain("palette-title");
    const meta = screen.getByText("feature · #392");
    expect(meta.className).toContain("palette-hint");
    expect(title.closest("li")?.className).toContain("palette-feature");
  });

  it("appends ` · landed` once the dossier reached the checkout", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([row({ landed: true })]);
    await searchFor("vetting");
    expect(await screen.findByText("feature · #392 · landed")).toBeTruthy();
  });

  it("opens the card detail on ↵ — live or landed", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([row({ landed: true })]);
    await searchFor("vetting");
    await screen.findByText("Plan — invite vetting");

    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "Enter" });

    await waitFor(() => expect(useUi.getState().boardDetailIssueId).toBe("i392"));
    // The detail lives in the project view's right slot, so the route has
    // to select the project and open the sheet — the same three moves
    // `runOpenCardDetail` makes.
    expect(useUi.getState().changesOpen).toBe(true);
    expect(useSidebar.getState().selectedProjectId).toBe("p1");
    expect(useUi.getState().paletteOpen).toBe(false);
  });

  it("falls back to the raw heading while the card is still resolving", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([]);
    await searchFor("vetting");
    await waitFor(() => expect(screen.getByText("Plan — 2026-08-07")).toBeTruthy());
    // No card, no `feature ·` meta claiming one.
    expect(screen.queryByText(/feature · #/)).toBeNull();
  });

  it("leaves non-feature rows exactly as they were", async () => {
    vi.mocked(search).mockResolvedValue([
      { itemType: "task", itemId: "t1", projectId: "p1", taskId: "t1", title: "navbar work" },
    ]);
    await searchFor("navbar");
    const title = await screen.findByText("navbar work");
    expect(title.closest("li")?.className).not.toContain("palette-feature");
    expect(screen.getByText("task")).toBeTruthy();
    expect(vi.mocked(dossierFeatureRows)).not.toHaveBeenCalled();
  });
});
