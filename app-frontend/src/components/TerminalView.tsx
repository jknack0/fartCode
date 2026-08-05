// Terminal tab (E2-12): attaches the task's live terminal session (xterm.js
// bound to a PTY-backed shell, owned by lib/terminals) to the pane. The PTY
// id IS the tab id. Unmount only detaches the DOM — task switches and tab
// flips keep the shell (and its scrollback) alive; only closing the tab or
// deleting the task kills it (tab store → killTerminal).
import { useEffect, useRef } from "react";
import { getTerminalSession } from "../lib/terminals";
import { terminalResize } from "../lib/tauri";
import "xterm/css/xterm.css";

export default function TerminalView({
  terminalId,
}: {
  terminalId: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const session = getTerminalSession(terminalId);
    container.appendChild(session.host);
    session.fit.fit();
    // Terminal-first task view: selecting a task must land the keyboard in
    // the shell immediately — no click required.
    session.term.focus();

    const resizeObserver = new ResizeObserver(() => {
      session.fit.fit();
      const { cols, rows } = session.term;
      void terminalResize(terminalId, cols, rows).catch(() => {});
    });
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      // Detach, never kill: the session + PTY belong to the TAB, and the
      // view remounts whenever the user comes back to this tab/task.
      session.host.remove();
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
