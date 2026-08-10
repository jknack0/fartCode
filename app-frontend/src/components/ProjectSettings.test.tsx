// Project settings pane — the dossier switch and the full-replace hazard
// it sits on (#74, handoff v3 §8e).
//
// §8e requires the consent decision to be reversible from settings "in
// both directions", which makes this row the escape hatch for a card that
// only ever fires once. The second test is the subtler one:
// `update_project_settings` is FULL-REPLACE, so a pane holding a snapshot
// from before the consent card was answered would send the stale value
// back and silently un-answer it — turning any unrelated toggle into a
// consent revocation and a re-prompt.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  getProjectSettings: vi.fn(),
  updateProjectSettings: vi.fn(),
  projectSettingsProvenance: vi.fn(() => Promise.resolve({})),
  projectSettingsShare: vi.fn(() => Promise.resolve()),
  setDefaultAgent: vi.fn(() => Promise.resolve()),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  hostDependencyList: vi.fn(() => Promise.resolve([])),
  hostDependencyRegistrySummary: vi.fn(() => Promise.resolve(null)),
  hostDependencyInstall: vi.fn(),
  hostDependencyUpdate: vi.fn(),
}));

import { ProjectSettingsPane } from "./ProjectSettings";
import {
  getProjectSettings,
  updateProjectSettings,
  type ProjectSettingsDto,
} from "../lib/tauri";
import { useDependencies } from "../store/dependencies";

const SETTINGS: ProjectSettingsDto = {
  tmux: false,
  featureDossiers: null,
  featureLogSeededVersion: 3,
};

/** The row's value cell, by its label. */
function rowValue(label: string): string {
  const row = screen.getByText(label).closest(".fc-set-row")!;
  return row.querySelector(".fc-set-value")!.textContent!.trim();
}

function clickRow(label: string): void {
  fireEvent.click(screen.getByText(label).closest(".fc-set-row")!);
}

async function renderPane(over: Partial<ProjectSettingsDto> = {}) {
  vi.mocked(getProjectSettings).mockResolvedValue({ ...SETTINGS, ...over });
  render(<ProjectSettingsPane projectId="p1" />);
  await waitFor(() => expect(screen.getByText("Feature dossiers")).toBeInTheDocument());
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(updateProjectSettings).mockImplementation((_p, s) => Promise.resolve(s));
  useDependencies.setState({ deps: [] });
});

describe("feature dossiers row", () => {
  // Unanswered reads `off` because off is what the app DOES with it — the
  // backend fails closed on null — so the row never claims a consent the
  // project has not given.
  it("shows off while the project has never been asked", async () => {
    await renderPane();
    expect(rowValue("Feature dossiers")).toBe("off");
  });

  it("shows the stored value in both directions", async () => {
    await renderPane({ featureDossiers: true });
    expect(rowValue("Feature dossiers")).toBe("on");
  });

  it("turns on, writing an explicit true", async () => {
    await renderPane();
    clickRow("Feature dossiers");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    expect(updateProjectSettings).toHaveBeenCalledWith("p1", {
      ...SETTINGS,
      featureDossiers: true,
    });
    await waitFor(() => expect(rowValue("Feature dossiers")).toBe("on"));
  });

  it("turns back off, writing an explicit false", async () => {
    await renderPane({ featureDossiers: true });
    clickRow("Feature dossiers");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    expect(updateProjectSettings).toHaveBeenCalledWith("p1", {
      ...SETTINGS,
      featureDossiers: false,
    });
    await waitFor(() => expect(rowValue("Feature dossiers")).toBe("off"));
  });
});

describe("full-replace safety", () => {
  // The pane's snapshot is from mount; the consent card answered later.
  // Toggling something unrelated must not send the stale null back.
  it("re-reads before writing, so a stale pane cannot revoke consent", async () => {
    await renderPane(); // loaded with featureDossiers: null
    // Meanwhile, the consent card answers and the scaffold is seeded.
    vi.mocked(getProjectSettings).mockResolvedValue({
      ...SETTINGS,
      featureDossiers: true,
      featureLogSeededVersion: 4,
    });

    clickRow("tmux terminals");
    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalled());
    const [, sent] = vi.mocked(updateProjectSettings).mock.calls[0];
    expect(sent.tmux).toBe(true); // the pane's own edit still wins
    expect(sent.featureDossiers).toBe(true); // NOT clobbered back to null
    expect(sent.featureLogSeededVersion).toBe(4);
  });

  // Re-reading is necessary but not sufficient: two quick toggles would
  // interleave, the second reading the row before the first had written
  // it, and its full-replace write would undo the first. The pair has to
  // be serialized, not merely fresh.
  it("serializes rapid toggles instead of letting them clobber each other", async () => {
    // A stand-in for the stored row, so the second commit's read can only
    // see what the first commit actually wrote.
    let stored: ProjectSettingsDto = { ...SETTINGS };
    vi.mocked(getProjectSettings).mockImplementation(() => Promise.resolve({ ...stored }));
    vi.mocked(updateProjectSettings).mockImplementation((_p, s) => {
      stored = { ...(s as ProjectSettingsDto) };
      return Promise.resolve({ ...stored });
    });

    render(<ProjectSettingsPane projectId="p1" />);
    await waitFor(() => expect(screen.getByText("Feature dossiers")).toBeInTheDocument());

    // Both clicks land inside one tick — no await between them.
    clickRow("tmux terminals");
    clickRow("Feature dossiers");

    await waitFor(() => expect(updateProjectSettings).toHaveBeenCalledTimes(2));
    // Neither edit was lost.
    expect(stored.tmux).toBe(true);
    expect(stored.featureDossiers).toBe(true);
    expect(stored.featureLogSeededVersion).toBe(3);
  });
});
