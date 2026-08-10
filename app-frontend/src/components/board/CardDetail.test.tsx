// Card-detail dossier group (#75, handoff v3 §8f).
//
// The load-bearing assertions are the three shapes a dossier can take:
// a full one (timeline + the agent's inset section), one whose agent
// skipped the append (timeline, NO inset — never a nag), and no dossier at
// all (no group whatsoever — declined consent and pre-E19 cards are not
// empty states). Plus j/k walking the sections, with the footer count.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../../lib/tauri", () => ({
  issueList: vi.fn(() => Promise.resolve([])),
  issueUpdate: vi.fn(),
  issueDelete: vi.fn(() => Promise.resolve()),
  issueDispatch: vi.fn(),
  issueLink: vi.fn(),
  issueUnlink: vi.fn(),
  dossierRead: vi.fn(() => Promise.resolve(null)),
  listProviders: vi.fn(() => Promise.resolve([])),
  acpStart: vi.fn(),
  terminalOpenAgent: vi.fn(),
  terminalWrite: vi.fn(),
  columnList: vi.fn(() => Promise.resolve([])),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@tauri-apps/plugin-shell", () => ({ open: vi.fn(() => Promise.resolve()) }));

import CardDetail from "./CardDetail";
import { dossierRead, issueList } from "../../lib/tauri";
import type { DossierDto, IssueDto } from "../../lib/tauri";
import { useColumns } from "../../store/columns";
import { useUi } from "../../store/ui";

function issue(over: Partial<IssueDto> = {}): IssueDto {
  return {
    id: "i1",
    projectId: "p1",
    title: "Admin resend on an active invite crashes",
    body: null,
    acceptance: [],
    lane: "in_progress",
    position: 0,
    provider: null,
    model: null,
    prdPath: null,
    prdSection: null,
    dossierPath: "docs/features/invite-vetting.md",
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

function dossier(over: Partial<DossierDto> = {}): DossierDto {
  return {
    path: "docs/features/invite-vetting.md",
    hostPath: "/tmp/wt/docs/features/invite-vetting.md",
    timeline: [
      {
        stamp: "2026-08-06 09:00",
        at: "2026-08-06T09:00:00Z",
        text: "created · proposal · docs/prds/invite-vetting.md",
        running: false,
      },
      {
        stamp: "2026-08-07 10:00",
        at: "2026-08-07T10:00:00Z",
        text: "Plan · fable · launched → settled · 41m",
        running: false,
      },
    ],
    sections: [
      { heading: "Plan — 2026-08-07", body: "Gate the send path, not accept." },
      {
        heading: "Implement — 2026-08-09",
        body: "Vetting lives in the send interceptor.",
      },
    ],
    ...over,
  };
}

async function renderDetail() {
  render(<CardDetail projectId="p1" issueId="i1" />);
  await waitFor(() => expect(screen.getByLabelText("Issue title")).toBeTruthy());
}

beforeEach(() => {
  useColumns.setState({ byProject: {} });
  useUi.setState({ boardDetailIssueId: "i1" });
  vi.mocked(issueList).mockResolvedValue([issue()]);
  vi.mocked(dossierRead).mockResolvedValue(null);
});

describe("dossier group", () => {
  it("renders the path link, the timeline, and the focused section", async () => {
    vi.mocked(dossierRead).mockResolvedValue(dossier());
    await renderDetail();

    const group = await screen.findByLabelText("Dossier");
    expect(group.querySelector("h3")?.textContent).toBe("Dossier");
    // The path is the group's one link out.
    const link = screen.getByText("docs/features/invite-vetting.md");
    expect(link.className).toContain("card-detail-link");

    // Timeline: app-written breadcrumbs, date prefix split out so it can
    // render at --meta while the fact renders at --text-mid.
    const rows = group.querySelectorAll(".card-detail-timeline li");
    expect(rows.length).toBe(2);
    expect(rows[0].textContent).toContain("created · proposal");
    expect(rows[1].textContent).toContain("Plan · fable · launched → settled · 41m");
    expect(
      group.querySelectorAll(".card-detail-timeline-date").length,
    ).toBe(2);

    // Inset card: the agent's own words for the newest step, with the
    // heading rendered as the section header it is.
    expect(screen.getByText("## Implement — 2026-08-09")).toBeTruthy();
    expect(screen.getByText("Vetting lives in the send interceptor.")).toBeTruthy();
    expect(screen.getByText("2 sections · j k walk · ⌘K finds them")).toBeTruthy();
  });

  it("renders the timeline and NO inset section when the agent skipped the append", async () => {
    vi.mocked(dossierRead).mockResolvedValue(dossier({ sections: [] }));
    await renderDetail();

    const group = await screen.findByLabelText("Dossier");
    expect(group.querySelectorAll(".card-detail-timeline li").length).toBe(2);
    expect(group.querySelector(".card-detail-dossier-section")).toBeNull();
    // Never a nag: nothing tells the user a section is missing.
    expect(group.textContent).not.toMatch(/section/i);
  });

  it("renders no dossier group at all when the card has none", async () => {
    vi.mocked(dossierRead).mockResolvedValue(null);
    await renderDetail();
    await waitFor(() => expect(screen.getByText("Acceptance")).toBeTruthy());
    expect(screen.queryByLabelText("Dossier")).toBeNull();
  });

  it("renders `running · <elapsed>` for the step that has not settled", async () => {
    const started = new Date(Date.now() - 4 * 60_000).toISOString();
    vi.mocked(dossierRead).mockResolvedValue(
      dossier({
        timeline: [
          {
            stamp: started.slice(0, 16).replace("T", " "),
            at: started,
            text: "Implement · claude",
            running: true,
          },
        ],
      }),
    );
    await renderDetail();

    const now = await screen.findByText(/running · 4m/);
    expect(now.className).toContain("card-detail-timeline-now");
  });

  it("walks sections with j/k and keeps the footer count honest", async () => {
    vi.mocked(dossierRead).mockResolvedValue(dossier());
    await renderDetail();
    const group = await screen.findByLabelText("Dossier");

    // Focus starts on the newest section — the step that just settled.
    expect(screen.getByText("## Implement — 2026-08-09")).toBeTruthy();

    fireEvent.keyDown(group, { key: "k" });
    await waitFor(() => expect(screen.getByText("## Plan — 2026-08-07")).toBeTruthy());
    expect(screen.getByText("Gate the send path, not accept.")).toBeTruthy();
    // The count is the number of sections, not the cursor.
    expect(screen.getByText("2 sections · j k walk · ⌘K finds them")).toBeTruthy();

    fireEvent.keyDown(group, { key: "k" });
    expect(screen.getByText("## Plan — 2026-08-07")).toBeTruthy();

    fireEvent.keyDown(group, { key: "j" });
    await waitFor(() =>
      expect(screen.getByText("## Implement — 2026-08-09")).toBeTruthy(),
    );
  });

  it("leaves j/k alone while the title is being typed in", async () => {
    vi.mocked(dossierRead).mockResolvedValue(dossier());
    await renderDetail();
    await screen.findByLabelText("Dossier");

    const typed = fireEvent.keyDown(screen.getByLabelText("Issue title"), { key: "k" });
    expect(screen.getByText("## Implement — 2026-08-09")).toBeTruthy();
    expect(typed).toBe(true); // not swallowed — typing is not a walk
  });

  /// The sheet owns j/k whenever it is showing a dossier. Bailing on a
  /// one-section dossier let the key fall through to the board's global
  /// handler and walk the cards behind the sheet.
  it("swallows j/k even when there is only one section to show", async () => {
    vi.mocked(dossierRead).mockResolvedValue(
      dossier({ sections: [{ heading: "Plan — 2026-08-07", body: "One only." }] }),
    );
    await renderDetail();
    const group = await screen.findByLabelText("Dossier");

    const bubbled = fireEvent.keyDown(group, { key: "j" });
    expect(bubbled).toBe(false); // preventDefault'd: the board never sees it
    expect(screen.getByText("## Plan — 2026-08-07")).toBeTruthy();
  });

  it("renders `running` with no duration when the stamp had no zone", async () => {
    vi.mocked(dossierRead).mockResolvedValue(
      dossier({
        timeline: [
          { stamp: "yesterday", at: null, text: "Implement · claude", running: true },
        ],
      }),
    );
    await renderDetail();
    const now = await screen.findByText(/running/);
    expect(now.textContent).toBe(" · running");
  });

  it("says `1 section` for a single one", async () => {
    vi.mocked(dossierRead).mockResolvedValue(
      dossier({ sections: [{ heading: "Plan — 2026-08-07", body: "One only." }] }),
    );
    await renderDetail();
    expect(
      await screen.findByText("1 section · j k walk · ⌘K finds them"),
    ).toBeTruthy();
  });
});
