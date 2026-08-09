// KeyChord (E14-01): modifier set + key, parsed from the human-readable
// format used by the default map and settings UI ("⌘⇧N", "Ctrl+Tab",
// "⌘⌥↑"). macOS-first: ⌘ is Cmd/Meta; on other platforms the same map
// renders with Ctrl (the PRD's ⌘→Ctrl rule is a formatting concern — the
// chords themselves always carry their real modifiers).

export interface KeyChord {
  meta: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  /** Normalized key: lowercase char, "Tab", "Enter", "Backspace", "Escape",
   * "ArrowUp"…, "F1"–"F12". */
  key: string;
}

export const IS_MAC =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "");

const MOD_TOKENS: Record<string, keyof Pick<KeyChord, "meta" | "ctrl" | "alt" | "shift">> = {
  "⌘": "meta",
  cmd: "meta",
  command: "meta",
  meta: "meta",
  "⌃": "ctrl",
  ctrl: "ctrl",
  control: "ctrl",
  "⌥": "alt",
  alt: "alt",
  option: "alt",
  "⇧": "shift",
  shift: "shift",
};

const KEY_ALIASES: Record<string, string> = {
  up: "ArrowUp",
  arrowup: "ArrowUp",
  "↑": "ArrowUp",
  down: "ArrowDown",
  arrowdown: "ArrowDown",
  "↓": "ArrowDown",
  left: "ArrowLeft",
  arrowleft: "ArrowLeft",
  "←": "ArrowLeft",
  right: "ArrowRight",
  arrowright: "ArrowRight",
  "→": "ArrowRight",
  esc: "Escape",
  return: "Enter",
  "↩": "Enter",
  delete: "Backspace",
  "⌫": "Backspace",
  space: " ",
};

function normalizeKey(raw: string): string {
  const lower = raw.toLowerCase();
  if (KEY_ALIASES[lower]) return KEY_ALIASES[lower];
  if (/^f([1-9]|1[0-2])$/.test(lower)) return raw.toUpperCase();
  if (/^arrow(up|down|left|right)$/.test(lower)) {
    return raw[0].toUpperCase() + raw.slice(1).toLowerCase();
  }
  return raw.length === 1 ? lower : raw;
}

/** Parses "⌘⇧N", "Ctrl+Shift+Tab", "⌘⌥↑" → KeyChord, or null. */
export function parseChord(input: string): KeyChord | null {
  const chord: KeyChord = { meta: false, ctrl: false, alt: false, shift: false, key: "" };
  // Split on "+", but keep symbol clusters ("⌘⇧N") as one token group.
  const parts = input
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length === 0) return null;

  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    const isLast = i === parts.length - 1;
    if (!isLast) {
      const mod = MOD_TOKENS[part.toLowerCase()] ?? MOD_TOKENS[part];
      if (!mod) return null; // non-modifier before the end is malformed
      chord[mod] = true;
      continue;
    }
    // Last part: may be a symbol cluster (⌘⇧N) or a plain key.
    let rest = part;
    while (rest.length > 0) {
      const symbol = rest[0];
      const mod = MOD_TOKENS[symbol];
      if (mod && rest.length > 1) {
        chord[mod] = true;
        rest = rest.slice(1);
        continue;
      }
      break;
    }
    if (rest.length === 0) return null;
    chord.key = normalizeKey(rest);
  }
  if (!chord.key) return null;
  return chord;
}

const MAC_ORDER: Array<[keyof KeyChord, string]> = [
  ["meta", "⌘"],
  ["alt", "⌥"],
  ["shift", "⇧"],
  ["ctrl", "⌃"],
];
const PC_ORDER: Array<[keyof KeyChord, string]> = [
  ["ctrl", "Ctrl"],
  ["alt", "Alt"],
  ["shift", "Shift"],
  ["meta", "Meta"],
];

const KEY_GLYPHS: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Backspace: "⌫",
  Escape: "Esc",
  Tab: "Tab",
  Enter: "↩",
  " ": "Space",
};

/** Human-readable form: "⌘⇧N" on macOS, "Ctrl+Shift+N" elsewhere. */
export function formatChord(chord: KeyChord): string {
  const order = IS_MAC ? MAC_ORDER : PC_ORDER;
  const mods = order.filter(([m]) => chord[m]).map(([, label]) => label);
  const key = KEY_GLYPHS[chord.key] ?? (chord.key.length === 1 ? chord.key.toUpperCase() : chord.key);
  return IS_MAC ? [...mods, key].join("") : [...mods, key].join("+");
}

export function chordsEqual(a: KeyChord, b: KeyChord): boolean {
  return (
    a.meta === b.meta &&
    a.ctrl === b.ctrl &&
    a.alt === b.alt &&
    a.shift === b.shift &&
    a.key === b.key
  );
}

/** US-layout shifted symbols → their base key. Bindings are written in
 * base form ("⌘⇧1", "⌘⇧."), but engines differ on whether e.key reports
 * the shifted character while a modifier chord is held — normalize so
 * both report shapes match the same stored chord. */
const SHIFTED_KEYS: Record<string, string> = {
  "!": "1", "@": "2", "#": "3", "$": "4", "%": "5",
  "^": "6", "&": "7", "*": "8", "(": "9", ")": "0",
  "_": "-", "+": "=", "{": "[", "}": "]", "|": "\\",
  ":": ";", '"': "'", "<": ",", ">": ".", "?": "/", "~": "`",
};

/** Builds the chord for a KeyboardEvent (DOM or React synthetic). */
export function chordFromEvent(e: {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  key: string;
}): KeyChord {
  const key = normalizeKey(e.key);
  return {
    meta: e.metaKey,
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    key: e.shiftKey && SHIFTED_KEYS[key] ? SHIFTED_KEYS[key] : key,
  };
}

/** Modifier-bearing or special keys only — the registry never binds plain
 * printable characters (they always belong to the focused element). */
export function isBindableChord(chord: KeyChord): boolean {
  if (chord.meta || chord.ctrl || chord.alt) return true;
  return ["Tab", "Escape", "Backspace", "Enter"].includes(chord.key) || /^F\d{1,2}$/.test(chord.key);
}
