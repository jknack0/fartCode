// Terminal tab (E2-12): xterm.js bound to a PTY-backed shell in the task's
// workspace. The PTY id IS the tab id (created by the new-terminal command).
// Output/exit arrive as Tauri events; input goes through terminal_write.
import { useEffect, useRef } from "react";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";
import "xterm/css/xterm.css";
import {
  onTerminalExited,
  onTerminalOutput,
  terminalClose,
  terminalResize,
  terminalWrite,
} from "../lib/tauri";

// PTY lifecycle is reference-counted OUTSIDE the React effect: StrictMode
// runs dev effects as mount → cleanup → remount, so a close fired from
// cleanup would kill the shell before the remounted view could use it.
// Close only when the LAST view lets go — deferred one beat so an immediate
// remount cancels the pending close.
const ptyRefs = new Map<string, number>();
const pendingCloses = new Map<string, number>();

function acquirePty(id: string): void {
  const pending = pendingCloses.get(id);
  if (pending !== undefined) {
    clearTimeout(pending);
    pendingCloses.delete(id);
  }
  ptyRefs.set(id, (ptyRefs.get(id) ?? 0) + 1);
}

function releasePty(id: string): void {
  const refs = (ptyRefs.get(id) ?? 1) - 1;
  if (refs > 0) {
    ptyRefs.set(id, refs);
    return;
  }
  ptyRefs.delete(id);
  pendingCloses.set(
    id,
    window.setTimeout(() => {
      pendingCloses.delete(id);
      // Releasing the last view kills the shell (⌘W, split collapse, task
      // switch teardown all funnel through unmount). Errors are fine — the
      // PTY may already be gone (restart-survival: tabs outlive shells).
      void terminalClose(id).catch(() => {});
    }, 250),
  );
}

function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export default function TerminalView({
  terminalId,
}: {
  terminalId: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      // Nerd Fonts first so Powerline/nerd glyphs (, , icons) render;
      // machines without them fall back to the system mono stack.
      fontFamily:
        '"JetBrainsMono Nerd Font Mono", "MesloLGS Nerd Font Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 12,
      theme: {
        background: "#0d1424",
        foreground: "#e5e7eb",
        cursor: "#d97706",
        selectionBackground: "rgba(217, 119, 6, 0.3)",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);
    fit.fit();
    // Terminal-first task view: selecting a task must land the keyboard in
    // the shell immediately — no click required.
    term.focus();

    acquirePty(terminalId);

    let disposed = false;
    let exited = false;

    const unsubs: Array<() => void> = [];
    void onTerminalOutput(({ terminalId: id, data }) => {
      if (id === terminalId && !disposed && !exited) {
        term.write(base64ToBytes(data));
      }
    }).then((un) => unsubs.push(un));
    void onTerminalExited(({ terminalId: id, exitCode }) => {
      if (id === terminalId && !disposed) {
        exited = true;
        term.write(
          `\r\n[process exited${exitCode !== null ? ` with code ${exitCode}` : ""}] — close this tab\r\n`,
        );
      }
    }).then((un) => unsubs.push(un));

    const onData = term.onData((data) => {
      if (!exited) void terminalWrite(terminalId, data).catch(() => {});
    });

    const resizeObserver = new ResizeObserver(() => {
      if (disposed) return;
      fit.fit();
      const { cols, rows } = term;
      void terminalResize(terminalId, cols, rows).catch(() => {});
    });
    resizeObserver.observe(container);

    return () => {
      disposed = true;
      onData.dispose();
      resizeObserver.disconnect();
      unsubs.forEach((un) => un());
      term.dispose();
      releasePty(terminalId);
    };
  }, [terminalId]);

  // Keyboard focus: clicking the terminal surface focuses xterm so typing
  // lands in the shell even when another element held focus.
  return (
    <div
      className="terminal-container"
      ref={containerRef}
      onClick={(e) => {
        // xterm's key input lands on its hidden helper textarea, not the
        // visible surface — focus that so typing reaches the shell.
        const target = e.currentTarget.querySelector(
          ".xterm-helper-textarea",
        ) as HTMLElement | null;
        target?.focus();
      }}
    />
  );
}
