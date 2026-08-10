// ⌘K feature hits (#75, handoff v3 §8h).
//
// #72 wrote one `feature` row per dossier section but the command filtered
// them out of the palette; #75 deletes that filter, so the first thing this
// suite proves is that a feature row APPEARS at all. Then the §8h shape:
// `<Column> — <feature title>` on the left, mono `feature · #id` on the
// right, and ↵ opening the CARD DETAIL — one destination whether the
// feature is live or landed, INCLUDING while (or if) the title lookup
// never resolves.
//
// §8h's ` · landed` suffix (#83): a committed-content answer against the
// base ref drives it — rendered on a literal `true` only, since unknown is
// never a guess (see FeatureRowDto in fartcode-app/src/commands/dossiers.rs).

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
  // The card ↵ opens rides WITH the hit (resolved backend-side from the
  // item id), so routing never waits on the title lookup.
  issueId: "i392",
};

function row(over: Partial<FeatureRowDto> = {}): FeatureRowDto {
  return {
    itemId: FEATURE_HIT.itemId,
    issueId: "i392",
    title: "#392 invite vetting",
    externalRef: null,
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

  it("opens the card detail on ↵ — live or landed", async () => {
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

  /// The routing race: keying the §8h branch on the TITLE lookup dropped
  /// the row into the project/task runner, which matches neither — so ↵
  /// closed the palette and opened nothing. The hit's own `issueId` makes
  /// the row routable from the first frame.
  /// #83: ` · landed` rides the right meta, and only on a literal `true` —
  /// a definitive no AND an unknown answer both render nothing.
  it("appends ` · landed` only on a true ancestry answer", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([
      row({ externalRef: "https://github.com/o/r/issues/392", landed: true }),
    ]);
    await searchFor("vetting");
    await screen.findByText("feature · #392 · landed");
  });

  it("renders no landed tag on a definitive no or an unknown answer", async () => {
    for (const landed of [false, null, undefined] as const) {
      vi.mocked(dossierFeatureRows).mockResolvedValue([
        row({ externalRef: "https://github.com/o/r/issues/392", landed }),
      ]);
      const { unmount } = render(<CommandPalette />);
      fireEvent.change(screen.getByLabelText("Command palette"), {
        target: { value: "vetting" },
      });
      await screen.findByText("feature · #392");
      expect(screen.queryByText(/landed/)).toBeNull();
      unmount();
    }
  });

  it("still opens the card detail when the title lookup returns nothing", async () => {
    vi.mocked(dossierFeatureRows).mockResolvedValue([]);
    await searchFor("vetting");
    // Falls back to the indexed heading — it is what the row matched on.
    await waitFor(() => expect(screen.getByText("Plan — 2026-08-07")).toBeTruthy());

    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "Enter" });

    await waitFor(() => expect(useUi.getState().boardDetailIssueId).toBe("i392"));
    expect(useUi.getState().paletteOpen).toBe(false);
  });

  it("still opens the card detail when the title lookup rejects", async () => {
    vi.mocked(dossierFeatureRows).mockRejectedValue(new Error("ipc down"));
    await searchFor("vetting");
    await waitFor(() => expect(screen.getByText("Plan — 2026-08-07")).toBeTruthy());

    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "Enter" });
    await waitFor(() => expect(useUi.getState().boardDetailIssueId).toBe("i392"));
  });

  /// A row nothing knows how to open must leave the palette standing —
  /// dismissing it makes ↵ look like it worked and eats the keystroke.
  it("leaves the palette open when ↵ has nowhere to go", async () => {
    vi.mocked(search).mockResolvedValue([
      {
        itemType: "prd",
        itemId: "docs/prds/x.md",
        projectId: "p1",
        taskId: null,
        title: "docs/prds/x.md",
        issueId: null,
      },
    ]);
    await searchFor("prds");
    await screen.findByText("docs/prds/x.md");

    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByLabelText("Command palette"), { key: "Enter" });
    expect(useUi.getState().paletteOpen).toBe(true);
    expect(useUi.getState().boardDetailIssueId).toBeNull();
  });

  it("leaves non-feature rows exactly as they were", async () => {
    vi.mocked(search).mockResolvedValue([
      {
        itemType: "task",
        itemId: "t1",
        projectId: "p1",
        taskId: "t1",
        title: "navbar work",
        issueId: null,
      },
    ]);
    await searchFor("navbar");
    const title = await screen.findByText("navbar work");
    expect(title.closest("li")?.className).not.toContain("palette-feature");
    expect(screen.getByText("task")).toBeTruthy();
    expect(vi.mocked(dossierFeatureRows)).not.toHaveBeenCalled();
  });
});
