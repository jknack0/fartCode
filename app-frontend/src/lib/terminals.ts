// Interactive terminal sessions (E2-12): xterm instances live OUTSIDE React,
// keyed by PTY id, so task switches and tab flips detach/reattach the same
// shell + scrollback instead of recreating either. PTY lifetime is owned by
// the tab store (closeTab / dropTask call killTerminal / disposeTerminalSession)
// — component cleanup only detaches the DOM, never kills the shell.
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  onTerminalExited,
  onTerminalOutput,
  terminalClose,
  terminalTail,
  terminalWrite,
} from "./tauri";

export interface TermSession {
  term: Terminal;
  fit: FitAddon;
  /** Detached/reattached by TerminalView; owns the xterm DOM. */
  host: HTMLDivElement;
  exited: boolean;
  dispose: () => void;
}

const sessions = new Map<string, TermSession>();

/** Reads the xterm theme from the design tokens in styles.css :root.
 * xterm's own color parser only takes hex/rgb(a) strings, so the tokens
 * keep those forms; fallbacks mirror the token values. */
function xtermTheme() {
  const root = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) =>
    root.getPropertyValue(name).trim() || fallback;
  return {
    background: v("--xterm-bg", "#101012"),
    foreground: v("--xterm-fg", "#e8e8ea"),
    cursor: v("--xterm-cursor", "#e8e8ea"),
    selectionBackground: v("--xterm-selection-bg", "rgba(110, 231, 168, 0.28)"),
    selectionForeground: v("--xterm-selection-fg", "#e8e8ea"),
    scrollbarSliderBackground: v("--xterm-scrollbar-thumb", "#383738"),
    scrollbarSliderHoverBackground: v("--xterm-scrollbar-thumb-hover", "#626162"),
    scrollbarSliderActiveBackground: v("--xterm-scrollbar-thumb-hover", "#626162"),
  };
}

export function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Lazily builds (or returns) the live session for a PTY id. */
export function getTerminalSession(terminalId: string): TermSession {
  const existing = sessions.get(terminalId);
  if (existing) return existing;

  const term = new Terminal({
    cursorBlink: true,
    // Nerd Fonts first so Powerline/nerd glyphs render; then the bundled
    // JetBrains Mono; last resort the system mono stack.
    fontFamily:
      '"JetBrainsMono Nerd Font Mono", "MesloLGS Nerd Font Mono", "JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, monospace',
    fontSize: 12,
    // DESIGN.md terminal token: 12px/1.6 (xterm defaults to 1.0 otherwise).
    lineHeight: 1.6,
    // Agent CLIs blow past xterm's 1000-line default constantly. At the cap
    // every new line trims one off the top, which drags a scrolled-up
    // viewport away from the bottom (xterm anchors to content) and makes
    // reflow-on-resize destructive.
    scrollback: 10_000,
    // Theme values come from styles.css :root (--xterm-*) so the design
    // tokens stay the single source; fallbacks match the current tokens.
    theme: xtermTheme(),
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const host = document.createElement("div");
  host.className = "terminal-surface";
  term.open(host);

  let disposed = false;
  let exited = false;
  let ready = false;
  const pending: Uint8Array[] = [];
  const unsubs: Array<() => void> = [];

  // Firehose coalescing: agent CLIs emit hundreds of tiny chunks a second,
  // and every term.write() costs a parse/render/viewport-sync cycle that
  // also races user scrolling (github/copilot-cli#1805). Queue chunks and
  // hand xterm one concatenated write per animation frame. Everything the
  // terminal shows goes through enqueue() so ordering stays FIFO.
  let flushHandle: number | null = null;
  const queue: Uint8Array[] = [];
  const flush = () => {
    flushHandle = null;
    if (disposed || queue.length === 0) return;
    let total = 0;
    for (const chunk of queue) total += chunk.length;
    const joined = new Uint8Array(total);
    let offset = 0;
    for (const chunk of queue) {
      joined.set(chunk, offset);
      offset += chunk.length;
    }
    queue.length = 0;
    term.write(joined);
  };
  const enqueue = (bytes: Uint8Array) => {
    queue.push(bytes);
    flushHandle ??= requestAnimationFrame(flush);
  };

  // Subscribe BEFORE fetching the scrollback tail so output arriving in
  // between is buffered, not lost; the tail then replays first (reattach
  // after a frontend reload — plain PTYs have no scrollback server).
  void (async () => {
    const un = await onTerminalOutput(({ terminalId: id, data }) => {
      if (id !== terminalId || disposed || exited) return;
      const bytes = base64ToBytes(data);
      if (ready) enqueue(bytes);
      else pending.push(bytes);
    });
    unsubs.push(un);
    const tail = await terminalTail(terminalId).catch(() => null);
    if (tail && !disposed) enqueue(base64ToBytes(tail));
    ready = true;
    for (const bytes of pending) enqueue(bytes);
    pending.length = 0;
  })();
  void onTerminalExited(({ terminalId: id, exitCode }) => {
    if (id === terminalId && !disposed) {
      exited = true;
      session.exited = true;
      enqueue(
        new TextEncoder().encode(
          `\r\n[process exited${exitCode !== null ? ` with code ${exitCode}` : ""}] — close this tab\r\n`,
        ),
      );
    }
  }).then((un) => unsubs.push(un));

  const onData = term.onData((data) => {
    if (!exited) void terminalWrite(terminalId, data).catch(() => {});
  });

  const session: TermSession = {
    term,
    fit,
    host,
    exited: false,
    dispose: () => {
      if (disposed) return;
      disposed = true;
      if (flushHandle !== null) cancelAnimationFrame(flushHandle);
      onData.dispose();
      unsubs.forEach((un) => un());
      term.dispose();
      sessions.delete(terminalId);
    },
  };
  sessions.set(terminalId, session);
  return session;
}

/** Frees the xterm session without touching the backend PTY (task delete
 * teardown — the backend's close_task already killed those shells). */
export function disposeTerminalSession(terminalId: string): void {
  sessions.get(terminalId)?.dispose();
}

/** Kills the shell behind a terminal tab and discards its session. */
export function killTerminal(terminalId: string): void {
  disposeTerminalSession(terminalId);
  // Errors are fine — the PTY may already be gone (shell exited on its own).
  void terminalClose(terminalId).catch(() => {});
}
