import { describe, it, expect } from "vitest";
import {
  IS_MAC,
  KeyChord,
  chordFromEvent,
  chordsEqual,
  formatChord,
  isBindableChord,
  parseChord,
} from "./keychord";

function chord(over: Partial<KeyChord> & { key: string }): KeyChord {
  return { meta: false, ctrl: false, alt: false, shift: false, ...over };
}

describe("parseChord", () => {
  it("parses a mac symbol cluster", () => {
    expect(parseChord("⌘⇧N")).toEqual(chord({ meta: true, shift: true, key: "n" }));
  });

  it("parses plus-separated chords, with case-insensitive modifier names", () => {
    expect(parseChord("Ctrl+Shift+Tab")).toEqual(chord({ ctrl: true, shift: true, key: "Tab" }));
    expect(parseChord("CTRL+shift+Tab")).toEqual(chord({ ctrl: true, shift: true, key: "Tab" }));
  });

  it("does NOT case-normalize multi-character key names outside KEY_ALIASES", () => {
    // Documented gap, not a wish: "tab"/"enter" have no alias entry, so they
    // survive verbatim and can never match chordFromEvent's "Tab"/"Enter".
    // The default keymap only ever spells these canonically; a lowercase user
    // override would silently never fire.
    expect(parseChord("ctrl+tab")?.key).toBe("tab");
    expect(parseChord("⌘enter")?.key).toBe("enter");
    expect(chordFromEvent({ metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, key: "Enter" }).key).toBe(
      "Enter",
    );
  });

  it("accepts every alias for a modifier", () => {
    for (const s of ["cmd+a", "command+a", "meta+a", "⌘A"]) {
      expect(parseChord(s)).toEqual(chord({ meta: true, key: "a" }));
    }
    for (const s of ["alt+a", "option+a", "⌥A"]) {
      expect(parseChord(s)).toEqual(chord({ alt: true, key: "a" }));
    }
    for (const s of ["control+a", "⌃A"]) {
      expect(parseChord(s)).toEqual(chord({ ctrl: true, key: "a" }));
    }
  });

  it("normalizes arrow glyphs and names", () => {
    expect(parseChord("⌘⌥↑")).toEqual(chord({ meta: true, alt: true, key: "ArrowUp" }));
    expect(parseChord("ctrl+down")).toEqual(chord({ ctrl: true, key: "ArrowDown" }));
    expect(parseChord("ctrl+ArrowLeft")).toEqual(chord({ ctrl: true, key: "ArrowLeft" }));
    expect(parseChord("→")).toEqual(chord({ key: "ArrowRight" }));
  });

  it("normalizes the named-key aliases", () => {
    expect(parseChord("esc")?.key).toBe("Escape");
    expect(parseChord("return")?.key).toBe("Enter");
    expect(parseChord("↩")?.key).toBe("Enter");
    expect(parseChord("delete")?.key).toBe("Backspace");
    expect(parseChord("⌫")?.key).toBe("Backspace");
    expect(parseChord("ctrl+space")?.key).toBe(" ");
  });

  it("upper-cases function keys and lower-cases single characters", () => {
    expect(parseChord("f5")?.key).toBe("F5");
    expect(parseChord("F12")?.key).toBe("F12");
    expect(parseChord("⌘N")?.key).toBe("n");
    expect(parseChord("⌘n")?.key).toBe("n");
  });

  it("keeps non-alphanumeric keys verbatim", () => {
    expect(parseChord("⌘\\")).toEqual(chord({ meta: true, key: "\\" }));
    expect(parseChord("⌘,")).toEqual(chord({ meta: true, key: "," }));
  });

  it("rejects empty and malformed input", () => {
    expect(parseChord("")).toBeNull();
    expect(parseChord("+")).toBeNull();
    expect(parseChord("   ")).toBeNull();
    // A non-modifier before the last segment is malformed.
    expect(parseChord("N+⌘")).toBeNull();
    expect(parseChord("hyper+a")).toBeNull();
  });

  it("does not treat a lone modifier glyph as a modifier", () => {
    // "⌘" alone parses as a key, not meta — and is then unbindable, which is
    // what keeps a bare modifier out of the keymap.
    const lone = parseChord("⌘");
    expect(lone).toEqual(chord({ key: "⌘" }));
    expect(isBindableChord(lone!)).toBe(false);
  });
});

describe("chordFromEvent", () => {
  const base = { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };

  it("copies the modifier flags", () => {
    expect(chordFromEvent({ ...base, metaKey: true, altKey: true, key: "k" })).toEqual(
      chord({ meta: true, alt: true, key: "k" }),
    );
  });

  it("maps shifted US punctuation back to its base key", () => {
    // Bindings are stored in base form ("⌘⇧1"); engines differ on whether
    // e.key reports "!" or "1" while shift is held.
    const cases: Array<[string, string]> = [
      ["!", "1"],
      [")", "0"],
      ["?", "/"],
      [">", "."],
      ["<", ","],
      [":", ";"],
      ["_", "-"],
      ["+", "="],
      ["|", "\\"],
      ["~", "`"],
      ["{", "["],
      ['"', "'"],
    ];
    for (const [reported, base_] of cases) {
      expect(chordFromEvent({ ...base, metaKey: true, shiftKey: true, key: reported })).toEqual(
        chord({ meta: true, shift: true, key: base_ }),
      );
    }
  });

  it("leaves shifted punctuation alone when shift is not held", () => {
    expect(chordFromEvent({ ...base, key: "!" })).toEqual(chord({ key: "!" }));
  });

  it("already-base keys survive the shift mapping unchanged", () => {
    expect(chordFromEvent({ ...base, metaKey: true, shiftKey: true, key: "1" })).toEqual(
      chord({ meta: true, shift: true, key: "1" }),
    );
  });

  it("does not rewrite letters under shift", () => {
    expect(chordFromEvent({ ...base, shiftKey: true, key: "A" })).toEqual(
      chord({ shift: true, key: "a" }),
    );
  });

  it("normalizes special keys the same way parseChord does", () => {
    expect(chordFromEvent({ ...base, key: "ArrowUp" }).key).toBe("ArrowUp");
    expect(chordFromEvent({ ...base, key: "Escape" }).key).toBe("Escape");
    expect(chordFromEvent({ ...base, key: " " }).key).toBe(" ");
  });

  it("agrees with parseChord on the same physical chord", () => {
    expect(chordFromEvent({ ...base, metaKey: true, shiftKey: true, key: "N" })).toEqual(
      parseChord("⌘⇧N"),
    );
    // The shifted-punctuation case is exactly why the normalization exists.
    expect(chordFromEvent({ ...base, metaKey: true, shiftKey: true, key: "!" })).toEqual(
      parseChord("⌘⇧1"),
    );
  });
});

describe("formatChord", () => {
  it("renders modifiers in platform order", () => {
    const c = chord({ meta: true, ctrl: true, alt: true, shift: true, key: "a" });
    expect(formatChord(c)).toBe(IS_MAC ? "⌘⌥⇧⌃A" : "Ctrl+Alt+Shift+Meta+A");
  });

  it("joins without separators on mac and with '+' elsewhere", () => {
    const c = chord({ ctrl: true, shift: true, key: "Tab" });
    expect(formatChord(c)).toBe(IS_MAC ? "⇧⌃Tab" : "Ctrl+Shift+Tab");
  });

  it("renders key glyphs", () => {
    expect(formatChord(chord({ key: "ArrowUp" }))).toBe("↑");
    expect(formatChord(chord({ key: "Escape" }))).toBe("Esc");
    expect(formatChord(chord({ key: "Backspace" }))).toBe("⌫");
    expect(formatChord(chord({ key: "Enter" }))).toBe("↩");
    expect(formatChord(chord({ key: " " }))).toBe("Space");
  });

  it("upper-cases single-character keys and leaves named keys alone", () => {
    expect(formatChord(chord({ key: "n" }))).toBe("N");
    expect(formatChord(chord({ key: "F5" }))).toBe("F5");
  });

  it("round-trips through parseChord", () => {
    for (const s of ["⌘⇧N", "ctrl+tab", "⌘⌥↑", "⌘\\", "f5"]) {
      const parsed = parseChord(s)!;
      expect(parseChord(formatChord(parsed))).toEqual(parsed);
    }
  });
});

describe("chordsEqual", () => {
  it("compares every modifier and the key", () => {
    const a = chord({ meta: true, key: "n" });
    expect(chordsEqual(a, chord({ meta: true, key: "n" }))).toBe(true);
    expect(chordsEqual(a, chord({ meta: true, shift: true, key: "n" }))).toBe(false);
    expect(chordsEqual(a, chord({ ctrl: true, key: "n" }))).toBe(false);
    expect(chordsEqual(a, chord({ meta: true, alt: true, key: "n" }))).toBe(false);
    expect(chordsEqual(a, chord({ meta: true, key: "m" }))).toBe(false);
  });
});

describe("isBindableChord", () => {
  it("accepts anything carrying meta, ctrl or alt", () => {
    expect(isBindableChord(chord({ meta: true, key: "a" }))).toBe(true);
    expect(isBindableChord(chord({ ctrl: true, key: "a" }))).toBe(true);
    expect(isBindableChord(chord({ alt: true, key: "a" }))).toBe(true);
  });

  it("rejects plain printable characters — the focused element owns them", () => {
    expect(isBindableChord(chord({ key: "a" }))).toBe(false);
    expect(isBindableChord(chord({ key: "1" }))).toBe(false);
    expect(isBindableChord(chord({ key: " " }))).toBe(false);
    // Shift alone is not enough: ⇧A is just a capital letter.
    expect(isBindableChord(chord({ shift: true, key: "a" }))).toBe(false);
  });

  it("accepts the special keys and function keys with no modifier", () => {
    for (const key of ["Tab", "Escape", "Backspace", "Enter", "F1", "F12"]) {
      expect(isBindableChord(chord({ key }))).toBe(true);
    }
    expect(isBindableChord(chord({ key: "ArrowUp" }))).toBe(false);
  });
});
