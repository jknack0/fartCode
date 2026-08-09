import { describe, it, expect, vi, beforeEach } from "vitest";
import { IS_MAC } from "./keychord";
import {
  CommandDef,
  CommandId,
  Scope,
  ScopeContext,
  applyUserOverrides,
  createRegistry,
  dispatchKey,
  hintFor,
  listBindings,
  registerCommand,
  resetToDefaults,
} from "./registry";

type Registry = ReturnType<typeof createRegistry>;

/** Registers a command; `run` is a spy so tests can assert what fired. */
function cmd(
  registry: Registry,
  id: string,
  scope: Scope,
  defaultKeys: string[],
  opts: { skipInEditor?: boolean } = {},
): ReturnType<typeof vi.fn> {
  const run = vi.fn();
  registerCommand(registry, {
    id: id as CommandId,
    label: id,
    defaultKeys,
    scope,
    ...opts,
    run,
  } as CommandDef);
  return run;
}

/** A KeyboardEvent-shaped stub — dispatchKey only reads these five fields. */
function key(
  k: string,
  mods: { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean; repeat?: boolean } = {},
): KeyboardEvent & { preventDefault: ReturnType<typeof vi.fn> } {
  return {
    key: k,
    metaKey: !!mods.meta,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    shiftKey: !!mods.shift,
    repeat: !!mods.repeat,
    preventDefault: vi.fn(),
  } as unknown as KeyboardEvent & { preventDefault: ReturnType<typeof vi.fn> };
}

function ctx(over: Partial<ScopeContext> = {}): ScopeContext {
  return { projectView: false, taskView: false, editorFocused: false, modalOpen: false, ...over };
}

let registry: Registry;
beforeEach(() => {
  registry = createRegistry();
});

describe("registerCommand", () => {
  it("stores the parsed default chords", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    expect(registry.bindings["open-command-palette"]).toEqual([
      { meta: true, ctrl: false, alt: false, shift: false, key: "k" },
    ]);
  });

  it("drops unparseable and unbindable defaults", () => {
    // "x" alone is a printable character — never bindable.
    cmd(registry, "add-task", "global", ["not a chord", "x", "⌘N"]);
    expect(registry.bindings["add-task"]).toHaveLength(1);
    expect(registry.bindings["add-task"][0].key).toBe("n");
  });

  it("rejects a chord already bound in the same scope", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    cmd(registry, "add-task", "global", ["⌘N"]);
    cmd(registry, "new-project", "global", ["⌘N"]);
    expect(registry.bindings["add-task"]).toHaveLength(1);
    expect(registry.bindings["new-project"]).toHaveLength(0);
    expect(warn).toHaveBeenCalled();
  });

  it("allows the same chord in different scopes", () => {
    cmd(registry, "add-task", "global", ["⌘N"]);
    cmd(registry, "new-project", "task-view", ["⌘N"]);
    expect(registry.bindings["add-task"]).toHaveLength(1);
    expect(registry.bindings["new-project"]).toHaveLength(1);
  });

  it("ignores a duplicate command id", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    cmd(registry, "add-task", "global", ["⌘N"]);
    cmd(registry, "add-task", "global", ["⌘J"]);
    expect(registry.commands).toHaveLength(1);
    expect(registry.bindings["add-task"][0].key).toBe("n");
    expect(warn).toHaveBeenCalled();
  });
});

describe("dispatchKey — scope activation", () => {
  it("fires global and app-view commands with no view context", () => {
    const g = cmd(registry, "open-command-palette", "global", ["⌘K"]);
    const a = cmd(registry, "open-settings", "app-view", ["⌘,"]);
    expect(dispatchKey(registry, key("k", { meta: true }), ctx())).toBe("open-command-palette");
    expect(dispatchKey(registry, key(",", { meta: true }), ctx())).toBe("open-settings");
    expect(g).toHaveBeenCalledOnce();
    expect(a).toHaveBeenCalledOnce();
  });

  it("does not fire a task-view command outside the task view", () => {
    const run = cmd(registry, "resume-agent", "task-view", ["⌘J"]);
    expect(dispatchKey(registry, key("j", { meta: true }), ctx())).toBeNull();
    expect(run).not.toHaveBeenCalled();
    expect(dispatchKey(registry, key("j", { meta: true }), ctx({ taskView: true }))).toBe(
      "resume-agent",
    );
  });

  it("does not fire a project-view command outside the project view", () => {
    cmd(registry, "toggle-project-chat", "project-view", ["⌘⇧2"]);
    expect(dispatchKey(registry, key("2", { meta: true, shift: true }), ctx())).toBeNull();
    expect(
      dispatchKey(registry, key("2", { meta: true, shift: true }), ctx({ projectView: true })),
    ).toBe("toggle-project-chat");
  });

  it("only fires editor-scope commands while an editor is focused", () => {
    cmd(registry, "send-context", "editor", ["⌘Enter"]);
    expect(dispatchKey(registry, key("Enter", { meta: true }), ctx())).toBeNull();
    expect(dispatchKey(registry, key("Enter", { meta: true }), ctx({ editorFocused: true }))).toBe(
      "send-context",
    );
  });

  it("only fires modal-scope commands while a modal is open", () => {
    cmd(registry, "close-modal", "modal", ["Escape"]);
    expect(dispatchKey(registry, key("Escape"), ctx())).toBeNull();
    expect(dispatchKey(registry, key("Escape"), ctx({ modalOpen: true }))).toBe("close-modal");
  });

  it("ignores auto-repeat", () => {
    const run = cmd(registry, "open-command-palette", "global", ["⌘K"]);
    expect(dispatchKey(registry, key("k", { meta: true, repeat: true }), ctx())).toBeNull();
    expect(run).not.toHaveBeenCalled();
  });

  it("returns null and leaves the event alone when nothing matches", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    const e = key("q", { meta: true });
    expect(dispatchKey(registry, e, ctx())).toBeNull();
    expect(e.preventDefault).not.toHaveBeenCalled();
  });

  it("calls preventDefault on a match", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    const e = key("k", { meta: true });
    dispatchKey(registry, e, ctx());
    expect(e.preventDefault).toHaveBeenCalledOnce();
  });

  it("matches any of a command's chords", () => {
    cmd(registry, "toggle-sidebar", "global", ["⌘B", "⌘\\"]);
    expect(dispatchKey(registry, key("b", { meta: true }), ctx())).toBe("toggle-sidebar");
    expect(dispatchKey(registry, key("\\", { meta: true }), ctx())).toBe("toggle-sidebar");
  });

  it("normalizes shifted punctuation before matching", () => {
    // Default is written "⌘⇧1"; the engine may report e.key as "!".
    cmd(registry, "jump-to-tab-1", "global", ["⌘⇧1"]);
    expect(dispatchKey(registry, key("!", { meta: true, shift: true }), ctx())).toBe(
      "jump-to-tab-1",
    );
  });
});

describe("dispatchKey — modal suppression", () => {
  it("suspends task-view and project-view while a modal is open", () => {
    const task = cmd(registry, "resume-agent", "task-view", ["⌘J"]);
    const project = cmd(registry, "toggle-project-chat", "project-view", ["⌘⇧2"]);
    const c = ctx({ taskView: true, projectView: true, modalOpen: true });
    expect(dispatchKey(registry, key("j", { meta: true }), c)).toBeNull();
    expect(dispatchKey(registry, key("2", { meta: true, shift: true }), c)).toBeNull();
    expect(task).not.toHaveBeenCalled();
    expect(project).not.toHaveBeenCalled();
  });

  it("leaves global and app-view live under a modal", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    expect(dispatchKey(registry, key("k", { meta: true }), ctx({ modalOpen: true }))).toBe(
      "open-command-palette",
    );
  });

  it("lets a modal command win over a global one bound to the same chord", () => {
    const modal = cmd(registry, "close-modal", "modal", ["⌘K"]);
    const global = cmd(registry, "open-command-palette", "global", ["⌘K"]);
    expect(dispatchKey(registry, key("k", { meta: true }), ctx({ modalOpen: true }))).toBe(
      "close-modal",
    );
    expect(modal).toHaveBeenCalledOnce();
    expect(global).not.toHaveBeenCalled();
  });
});

describe("dispatchKey — skipInEditor", () => {
  it("skips a skipInEditor command while an editor is focused", () => {
    const run = cmd(registry, "close-tab", "global", ["⌘W"], { skipInEditor: true });
    expect(dispatchKey(registry, key("w", { meta: true }), ctx({ editorFocused: true }))).toBeNull();
    expect(run).not.toHaveBeenCalled();
    expect(dispatchKey(registry, key("w", { meta: true }), ctx())).toBe("close-tab");
  });

  it("still fires commands without the flag while an editor is focused", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    expect(dispatchKey(registry, key("k", { meta: true }), ctx({ editorFocused: true }))).toBe(
      "open-command-palette",
    );
  });

  it("ignores skipInEditor inside the modal scope", () => {
    // A focused input in a dialog must not swallow the dialog's own keys.
    cmd(registry, "close-modal", "modal", ["Escape"], { skipInEditor: true });
    expect(dispatchKey(registry, key("Escape"), ctx({ modalOpen: true, editorFocused: true }))).toBe(
      "close-modal",
    );
  });

  it("falls through to a lower scope when the higher one is skipped", () => {
    cmd(registry, "close-tab", "task-view", ["⌘W"], { skipInEditor: true });
    const fallback = cmd(registry, "open-command-palette", "global", ["⌘W"]);
    expect(dispatchKey(registry, key("w", { meta: true }), ctx({ taskView: true, editorFocused: true }))).toBe(
      "open-command-palette",
    );
    expect(fallback).toHaveBeenCalledOnce();
  });
});

describe("dispatchKey — scope precedence", () => {
  it("orders modal > editor > task-view > project-view > app-view > global", () => {
    const order: Array<[Scope, string]> = [
      ["global", "open-command-palette"],
      ["app-view", "open-settings"],
      ["project-view", "toggle-project-chat"],
      ["task-view", "resume-agent"],
      ["editor", "send-context"],
      ["modal", "close-modal"],
    ];
    for (const [scope, id] of order) cmd(registry, id, scope, ["⌘K"]);

    const all = ctx({ projectView: true, taskView: true, editorFocused: true, modalOpen: true });
    expect(dispatchKey(registry, key("k", { meta: true }), all)).toBe("close-modal");
    // Without the modal, editor wins.
    expect(
      dispatchKey(
        registry,
        key("k", { meta: true }),
        ctx({ projectView: true, taskView: true, editorFocused: true }),
      ),
    ).toBe("send-context");
    // Without the editor, task-view wins over project-view.
    expect(
      dispatchKey(registry, key("k", { meta: true }), ctx({ projectView: true, taskView: true })),
    ).toBe("resume-agent");
    expect(dispatchKey(registry, key("k", { meta: true }), ctx({ projectView: true }))).toBe(
      "toggle-project-chat",
    );
    expect(dispatchKey(registry, key("k", { meta: true }), ctx())).toBe("open-settings");
  });
});

describe("applyUserOverrides", () => {
  beforeEach(() => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    cmd(registry, "add-task", "global", ["⌘N"]);
  });

  it("replaces the default chords and dispatches on the new one", () => {
    applyUserOverrides(registry, { "open-command-palette": ["⌘P"] });
    expect(dispatchKey(registry, key("p", { meta: true }), ctx())).toBe("open-command-palette");
    expect(dispatchKey(registry, key("k", { meta: true }), ctx())).toBeNull();
  });

  it("ignores unknown command ids", () => {
    applyUserOverrides(registry, { "no-such-command": ["⌘P"] });
    expect(dispatchKey(registry, key("k", { meta: true }), ctx())).toBe("open-command-palette");
  });

  it("ignores an override whose chords are all unparseable or unbindable", () => {
    applyUserOverrides(registry, { "open-command-palette": ["nonsense", "x"] });
    expect(registry.bindings["open-command-palette"][0].key).toBe("k");
  });

  it("ignores an override that collides within the same scope", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    applyUserOverrides(registry, { "open-command-palette": ["⌘N"] });
    expect(registry.bindings["open-command-palette"][0].key).toBe("k");
    expect(dispatchKey(registry, key("n", { meta: true }), ctx())).toBe("add-task");
    expect(warn).toHaveBeenCalled();
  });

  it("ignores a non-array value", () => {
    applyUserOverrides(registry, { "open-command-palette": "⌘P" as unknown as string[] });
    expect(registry.bindings["open-command-palette"][0].key).toBe("k");
  });

  it("supports multiple chords for one command", () => {
    applyUserOverrides(registry, { "open-command-palette": ["⌘P", "⌘⇧P"] });
    expect(dispatchKey(registry, key("p", { meta: true }), ctx())).toBe("open-command-palette");
    expect(dispatchKey(registry, key("p", { meta: true, shift: true }), ctx())).toBe(
      "open-command-palette",
    );
  });
});

describe("resetToDefaults", () => {
  it("restores default chords after an override", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    applyUserOverrides(registry, { "open-command-palette": ["⌘P"] });
    resetToDefaults(registry);
    expect(dispatchKey(registry, key("k", { meta: true }), ctx())).toBe("open-command-palette");
    expect(dispatchKey(registry, key("p", { meta: true }), ctx())).toBeNull();
  });
});

describe("hintFor / listBindings", () => {
  it("hints the first chord, and empty string when unbound", () => {
    cmd(registry, "toggle-sidebar", "global", ["⌘B", "⌘\\"]);
    cmd(registry, "add-task", "global", []);
    // The hint is the FIRST chord (⌘B), never the alternate (⌘\).
    expect(hintFor(registry, "toggle-sidebar" as CommandId)).toBe(IS_MAC ? "⌘B" : "Meta+B");
    expect(hintFor(registry, "add-task" as CommandId)).toBe("");
    expect(hintFor(registry, "never-registered" as CommandId)).toBe("");
  });

  it("flags only the customized commands, in registration order", () => {
    cmd(registry, "open-command-palette", "global", ["⌘K"]);
    cmd(registry, "add-task", "global", ["⌘N"]);
    applyUserOverrides(registry, { "add-task": ["⌘⇧N"] });

    const listed = listBindings(registry);
    expect(listed.map((b) => b.id)).toEqual(["open-command-palette", "add-task"]);
    expect(listed[0].customized).toBe(false);
    expect(listed[1].customized).toBe(true);
  });

  it("exposes run so callers can invoke a command without a keypress", () => {
    const run = cmd(registry, "open-command-palette", "global", ["⌘K"]);
    listBindings(registry)[0].run();
    expect(run).toHaveBeenCalledOnce();
  });
});
