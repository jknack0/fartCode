// Memory value dashboard (#76, handoff v3 §8g).
//
// The load-bearing assertions mirror the crate's inviolable rule: honest
// states render AS states. An empty project is four blanks (no "0
// re-explanations avoided", no invented 0%); a single landing is a point
// with no sparkline; the time-to-land caveat comes from the payload
// verbatim and is present in every rendering of the row.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("../lib/tauri", () => ({
  telemetryMemoryValue: vi.fn(),
}));

import { MemoryPane, formatDuration, sparkline } from "./MemoryPane";
import { telemetryMemoryValue } from "../lib/tauri";
import type { MemoryValueDto, TimeToLandKindDto } from "../lib/tauri";

const CAVEAT =
  "Trend, not attribution — your pipeline changed this month too. Cycle time moves for " +
  "many reasons; this chart cannot tell you which one.";

const SPARK_RE = /[▁▂▃▄▅▆▇█]/;

/** The exact payload fartcode_telemetry::memory::empty() serializes for a
 * fresh project: every signal on its own "not enough information" variant. */
function emptyPayload(): MemoryValueDto {
  return {
    projectId: "p1",
    windowDays: 90,
    windowSince: 0,
    citations: {
      sessions: 0,
      citedRead: 0,
      citedMention: 0,
      notCited: 0,
      unknown: 0,
      unknownWithHit: 0,
      wroteWithoutReading: 0,
    },
    reAsk: { kind: "unknown", stepsScanned: 0, stepsUnreadable: 0 },
    tokensSaved: {
      kind: "insufficient",
      citingWithUsage: 0,
      notCitingWithUsage: 0,
      neededPerArm: 3,
    },
    timeToLand: { caveat: CAVEAT, kind: { kind: "noData" } },
    sessionsObserved: 0,
    dossiersScanned: 0,
    cyclesOutsideWindow: 0,
    sectionsUndated: 0,
    clipped: false,
  };
}

function populatedPayload(timeToLand: TimeToLandKindDto): MemoryValueDto {
  return {
    ...emptyPayload(),
    citations: {
      sessions: 7,
      citedRead: 2,
      citedMention: 1,
      notCited: 3,
      unknown: 1,
      unknownWithHit: 1,
      wroteWithoutReading: 1,
    },
    reAsk: {
      kind: "observed",
      memoryAnswered: 7,
      humanAsked: 1,
      stepsTagged: 3,
      stepsScanned: 6,
      stepsUnreadable: 1,
    },
    tokensSaved: {
      kind: "estimated",
      perSession: 20_000,
      windowTotal: 60_000,
      citingMedian: 12_000,
      notCitingMedian: 32_000,
      citingSessions: 3,
      notCitingSessions: 3,
      basis: "contextWindowGauge",
    },
    timeToLand: { caveat: CAVEAT, kind: timeToLand },
    sessionsObserved: 7,
    dossiersScanned: 4,
  };
}

const mockValue = (payload: MemoryValueDto) =>
  vi.mocked(telemetryMemoryValue).mockResolvedValue(payload);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("MemoryPane", () => {
  it("renders an empty project as four honest blanks, never zeroes", async () => {
    mockValue(emptyPayload());
    const { container } = render(<MemoryPane projectId="p1" />);
    await screen.findByText("no memory signal yet");

    const text = container.textContent ?? "";
    // NOT a fabricated count headline.
    expect(text).not.toContain("re-explanation");
    expect(text).not.toContain("0 re-explanations avoided");
    // No invented percentage anywhere.
    expect(text).not.toMatch(/%/);
    // The honest row values.
    expect(text).toContain("unknown · 0 sessions, none attributable");
    expect(text).toContain("unknown · no tagged clarifications in 0 readable steps");
    expect(text).toContain("not enough data · 0 citing / 0 non-citing of 3 needed each");
    expect(text).toContain("no landed feature with both created and merged breadcrumbs yet");
    // The caveat, verbatim from the payload.
    expect(screen.getByText(CAVEAT)).toBeTruthy();
    // No sparkline glyphs for nothing.
    expect(text).not.toMatch(SPARK_RE);
    // Honesty footer.
    expect(text).toContain("0 sessions observed · 0 dossiers scanned");
  });

  it("renders a populated trend payload with headline, rows, and an ordered sparkline", async () => {
    mockValue(
      populatedPayload({
        kind: "trend",
        earlierMedianHours: 90,
        laterMedianHours: 25,
        landed: 4,
        landedHours: [100, 80, 30, 20],
      }),
    );
    const { container } = render(<MemoryPane projectId="p1" />);
    await screen.findByText("7 re-explanations avoided");

    const text = container.textContent ?? "";
    // Citations: 3 of 6 conclusive → 50%.
    expect(text).toContain("50% · 3 of 6 steps");
    expect(text).toContain("2 through a read or search, 1 named it only");
    expect(text).toContain("1 wrote their section without reading the file");
    // The excluded-but-cited count must travel with the exclusion — an
    // excluded session that named the dossier makes the rate a floor.
    expect(text).toContain("1 session excluded as unattributable (1 of them did name it)");
    // Re-ask: 1 of 8 tagged clarifications → 13%.
    expect(text).toContain("13% re-asked · 1 to you, 7 from memory · 3 tagged steps");
    expect(text).toContain("1 step could not be read");
    // Tokens.
    expect(text).toContain("≈60k lower · 3 vs 3 sessions");
    expect(text).toContain("estimated from a context-window gauge, not a billing figure");
    // Time to land: durations formatted, sparkline in landing order.
    expect(text).toContain("3.8d → 25.0h");
    expect(text).toContain("█▆▂▁");
    expect(text).toContain("4 landed");
    expect(screen.getByText(CAVEAT)).toBeTruthy();
    expect(text).toContain("7 sessions observed · 4 dossiers scanned");
  });

  it("renders a single landing as a point — no sparkline, no arrow, caveat intact", async () => {
    mockValue(populatedPayload({ kind: "singlePoint", hours: 30 }));
    const { container } = render(<MemoryPane projectId="p1" />);
    await screen.findByText("7 re-explanations avoided");

    const text = container.textContent ?? "";
    expect(text).toContain("30.0h · one landing — a point, not a trend");
    expect(text).not.toMatch(SPARK_RE);
    expect(text).not.toContain("→");
    expect(screen.getByText(CAVEAT)).toBeTruthy();
  });

  it("surfaces the clipped / undated / outside-window caveats in the footer", async () => {
    mockValue({
      ...emptyPayload(),
      sectionsUndated: 2,
      cyclesOutsideWindow: 1,
      clipped: true,
    });
    const { container } = render(<MemoryPane projectId="p1" />);
    await screen.findByText("no memory signal yet");
    const text = container.textContent ?? "";
    expect(text).toContain("2 undated sections excluded");
    expect(text).toContain("1 landing outside the window");
    expect(text).toContain("input clipped — figures are a floor");
  });

  it("shows a loading state then the window subline", async () => {
    mockValue(emptyPayload());
    render(<MemoryPane projectId="p1" />);
    expect(screen.getByText("loading…")).toBeTruthy();
    await waitFor(() =>
      expect(
        screen.getByText("last 90 days · computed locally, never leaves this machine"),
      ).toBeTruthy(),
    );
  });

  it("shows errors via the settings error style", async () => {
    vi.mocked(telemetryMemoryValue).mockRejectedValue(new Error("db locked"));
    render(<MemoryPane projectId="p1" />);
    await waitFor(() => expect(screen.getByText(/db locked/)).toBeTruthy());
  });
});

describe("sparkline", () => {
  it("normalizes min-max onto the eight glyphs, capped to the last 24", () => {
    expect(sparkline([100, 80, 30, 20])).toBe("█▆▂▁");
    expect(sparkline(Array.from({ length: 30 }, (_, i) => i))).toHaveLength(24);
    expect(sparkline([])).toBe("");
  });

  it("renders an all-equal series flat at the mid glyph — no invented slope", () => {
    expect(sparkline([5, 5, 5])).toBe("▄▄▄");
  });
});

describe("formatDuration", () => {
  it("uses hours under 48 and days above", () => {
    expect(formatDuration(30)).toBe("30.0h");
    expect(formatDuration(47.9)).toBe("47.9h");
    expect(formatDuration(48)).toBe("2.0d");
    expect(formatDuration(90)).toBe("3.8d");
  });
});
