// Terminal tab (E2-12): attaches the task's live terminal session (xterm.js
// bound to a PTY-backed shell, owned by lib/terminals) to the pane. The PTY
// id IS the tab id. Unmount only detaches the DOM — tab flips and task
// switches keep the shell (and its scrollback) alive; only closing the tab
// or deleting the task kills it (tab store → killTerminal).
import { useEffect, useRef } from "react";
import { getTerminalSession } from "../lib/terminals";
import { terminalResize } from "../lib/tauri";
import "@xterm/xterm/css/xterm.css";

export default function TerminalView({
  terminalId,
  active,
}: {
  terminalId: string;
  active: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const pokeRef = useRef<() => void>(() => {});

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const session = getTerminalSession(terminalId);
    container.appendChild(session.host);

    const sync = () => {
      // A refit reflows the whole buffer, and xterm anchors the viewport to
      // content, not to the bottom — so a resize (sidesheet toggle, split)
      // would detach a bottom-stuck terminal from its own tail. Pin it back.
      const buf = session.term.buffer.active;
      const atBottom = buf.viewportY === buf.baseY;
      session.fit.fit();
      if (atBottom) session.term.scrollToBottom();
      const { cols, rows } = session.term;
      void terminalResize(terminalId, cols, rows).catch(() => {});
    };

    // xterm measures char size off a DOM span, which is 0 while the host is
    // detached/hidden — getTerminalSession opens the term BEFORE attach, so
    // fit() silently no-ops and the PTY would sit at its 80x24 spawn size
    // until some unrelated layout change. xterm re-measures a frame after
    // becoming visible (IntersectionObserver); retry until that lands.
    let cancelled = false;
    const poke = () => {
      let retries = 0;
      const step = () => {
        if (cancelled) return;
        sync();
        const dims = session.fit.proposeDimensions();
        if ((!dims || Number.isNaN(dims.cols) || dims.cols <= 2) && retries++ < 600) {
          requestAnimationFrame(step);
        }
      };
      step();
    };
    pokeRef.current = poke;
    poke();
    session.term.focus();

    const resizeObserver = new ResizeObserver(sync);
    resizeObserver.observe(container);

    return () => {
      cancelled = true;
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
    // First activation of a hidden-mounted tab: refit now that it's visible.
    pokeRef.current();
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
