// Terminal tab (E2-12): attaches the task's live terminal session (xterm.js
// bound to a PTY-backed shell, owned by lib/terminals) to the pane. The PTY
// id IS the tab id. Unmount only detaches the DOM — tab flips and task
// switches keep the shell (and its scrollback) alive; only closing the tab
// or deleting the task kills it (tab store → killTerminal).
import { useEffect, useRef } from "react";
import { getTerminalSession } from "../lib/terminals";
import { terminalResize } from "../lib/tauri";
import "xterm/css/xterm.css";

export default function TerminalView({
  terminalId,
  active,
}: {
  terminalId: string;
  active: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const session = getTerminalSession(terminalId);
    container.appendChild(session.host);
    session.fit.fit();
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

  // Activation focus: panes keep every tab mounted (a tab switch must not
  // kill the PTY), so switching back to this tab re-focuses xterm's hidden
  // helper textarea — the keyboard lives in the shell (terminal-first).
  useEffect(() => {
    if (!active) return;
    const target = containerRef.current?.querySelector(
      ".xterm-helper-textarea",
    ) as HTMLElement | null;
    target?.focus();
  }, [active]);

  // Keyboard focus: clicking the terminal surface focuses xterm so typing
  // lands in the shell even when another element held focus.
  return (
    <div
      className="terminal-container"
      ref={containerRef}
      style={active ? undefined : { display: "none" }}
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
