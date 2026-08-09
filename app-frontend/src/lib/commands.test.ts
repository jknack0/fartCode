// The real keymap, checked against the registry's own conflict detection.
//
// `registerCommand` rejects a chord already bound in the same scope and
// warns — silently, from the app's point of view, since the command simply
// ends up unbound. So the guarantee worth testing is not "we think these
// are free" but "the registry accepted every one of them and said
// nothing", which is what these assert for the whole default map.

import { describe, it, expect, vi } from "vitest";

vi.mock("./tauri", () => ({
  hostDependencyList: vi.fn(() => Promise.resolve([])),
  terminalListForTask: vi.fn(() => Promise.resolve([])),
  terminalOpen: vi.fn(),
  terminalOpenAgent: vi.fn(),
  terminalOpenLifecycle: vi.fn(),
  terminalWrite: vi.fn(),
  terminalClose: vi.fn(),
  terminalTail: vi.fn(() => Promise.resolve("")),
  terminalSurviving: vi.fn(() => Promise.resolve([])),
  onTerminalExited: vi.fn(() => Promise.resolve(() => {})),
  onTerminalOutput: vi.fn(() => Promise.resolve(() => {})),
  onFartcodeEvent: vi.fn(() => Promise.resolve(() => {})),
  onAcpUpdate: vi.fn(() => Promise.resolve(() => {})),
  onAcpTranscript: vi.fn(() => Promise.resolve(() => {})),
  onAcpPermissionRequest: vi.fn(() => Promise.resolve(() => {})),
  acpStart: vi.fn(),
  acpStop: vi.fn(),
  acpCancel: vi.fn(),
  acpHistory: vi.fn(() => Promise.resolve([])),
  acpSendPrompt: vi.fn(),
  acpResolvePermission: vi.fn(),
  listConversations: vi.fn(() => Promise.resolve([])),
  listProjectConversations: vi.fn(() => Promise.resolve([])),
  getOrCreateProjectConversation: vi.fn(),
  createConversation: vi.fn(),
  listProviders: vi.fn(() => Promise.resolve([])),
  listProjects: vi.fn(() => Promise.resolve([])),
  listTasks: vi.fn(() => Promise.resolve([])),
  createTask: vi.fn(),
  createProject: vi.fn(),
  deleteProject: vi.fn(),
  deleteTask: vi.fn(),
  togglePin: vi.fn(),
  projectGitPull: vi.fn(() => Promise.resolve()),
  gitAddRemote: vi.fn(),
  gitCommit: vi.fn(),
  gitCommitState: vi.fn(() => Promise.resolve({ branch: null })),
  gitFetch: vi.fn(),
  gitPublish: vi.fn(),
  gitPull: vi.fn(),
  gitPush: vi.fn(),
  issueList: vi.fn(() => Promise.resolve([])),
  issueEnterColumn: vi.fn(),
  stepConfirm: vi.fn(),
  columnList: vi.fn(() => Promise.resolve([])),
  hostDependencyRegistrySummary: vi.fn(() => Promise.resolve(null)),
  hostDependencyInstall: vi.fn(),
  hostDependencyUpdate: vi.fn(),
  getProjectSettings: vi.fn(() => Promise.resolve({ scripts: {} })),
  setViewState: vi.fn(() => Promise.resolve()),
  getViewState: vi.fn(() => Promise.resolve(null)),
}));

import { registerAllCommands, registry } from "./commands";
import { formatChord, parseChord } from "./keychord";
import { listBindings, type CommandId, type Scope } from "./registry";

const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
registerAllCommands();

/** The pipeline commands this round added, with the chords they claim. */
const PIPELINE: Array<[CommandId, string, Scope]> = [
  ["advance-step", "⌘⇧→", "task-view"],
  ["confirm-step", "⌘⇧D", "task-view"],
  ["move-to-column", "⌘⇧M", "task-view"],
  ["open-card-detail", "⌘⇧I", "task-view"],
];

describe("the default keymap", () => {
  it("registers without a single rejected chord", () => {
    expect(warn).not.toHaveBeenCalled();
  });

  it("binds no chord twice inside one scope", () => {
    const seen = new Map<string, CommandId>();
    for (const b of listBindings(registry)) {
      for (const chord of b.chords) {
        const slot = `${b.scope}::${formatChord(chord)}`;
        expect(seen.has(slot), `${slot} claimed twice`).toBe(false);
        seen.set(slot, b.id);
      }
    }
  });
});

describe("pipeline chords", () => {
  it.each(PIPELINE)("%s keeps %s", (id, chord, scope) => {
    const binding = listBindings(registry).find((b) => b.id === id);
    expect(binding).toBeDefined();
    expect(binding!.scope).toBe(scope);
    // A rejected chord leaves the command bound to nothing at all.
    expect(binding!.chords).toHaveLength(1);
    expect(binding!.chords[0]).toEqual(parseChord(chord));
  });

  it("collides with nothing in the reference keymap", () => {
    // FLOWS.md §3.5 + design_handoff_v2: the chords the app already
    // promises, whatever scope they live in.
    const reference = [
      "⌘T", "⌘⇧T", "⌘J", "⌘.", "⌘D", "⌘W", "⌘N", "⌘K", "⌘⌫", "⌘\\",
      "⌘⇧1", "⌘⇧2", "⌘⇧.", "⌘⌥↑", "⌘⌥↓", "⌘↵", "⌘⇧A", "⌘⇧O",
      "⌘1", "⌘2", "⌘3", "⌘4", "⌘5", "⌘6", "⌘7", "⌘8", "⌘9",
    ].map((c) => formatChord(parseChord(c)!));
    for (const [, chord] of PIPELINE) {
      expect(reference).not.toContain(formatChord(parseChord(chord)!));
    }
  });

  it("avoids the ⌘⇧3/4/5 range macOS takes for screenshots", () => {
    const taken = ["⌘⇧3", "⌘⇧4", "⌘⇧5"].map((c) => formatChord(parseChord(c)!));
    for (const [, chord] of PIPELINE) {
      expect(taken).not.toContain(formatChord(parseChord(chord)!));
    }
  });

  it("skips the three that an editor would otherwise swallow", () => {
    const byId = new Map(registry.commands.map((c) => [c.id, c]));
    // ⌘⇧→ is select-to-end-of-line in a text field; a dispatch or a move
    // fired from a composer is a surprise. The card detail is navigation,
    // like its ⌘⇧1/⌘⇧2 siblings on the same panel slot, so it stays live.
    expect(byId.get("advance-step")?.skipInEditor).toBe(true);
    expect(byId.get("confirm-step")?.skipInEditor).toBe(true);
    expect(byId.get("move-to-column")?.skipInEditor).toBe(true);
    expect(byId.get("open-card-detail")?.skipInEditor).toBeUndefined();
  });
});
