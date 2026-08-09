# fartCode nav + board — React

Drop-in TSX, no CSS file, no dependencies beyond React. Styling is inline objects built
from `theme.ts` so it can't drift from the design.

    theme.ts          colors, type, widths. the only place values live
    StatusDot.tsx     the whole status vocabulary + the two @keyframes
    LeftRail.tsx      56px icon rail: projects, +, sessions, settings
    ProjectFlyout.tsx 244px flyout: name, path · branch, in-flight work only
    TaskCard.tsx      boxless card + Column header
    Composer.tsx      one field; leading ">" makes it a session instead of a task

## Wiring

    <div style={{ display: 'flex', height: '100vh', background: c.window, color: c.text, fontFamily: sans }}>
      <style>{keyframes}</style>
      <LeftRail projects={projects} activeId={active} view="board" … />
      {navOpen && <ProjectFlyout name="ade" path="…/Dev/ade" branch="main" tasks={live} … />}
      <main style={{ flex: 1, minWidth: 0, display: 'flex' }}>…columns…</main>
    </div>

## Rules the code encodes

- `tasks` passed to the flyout must be in-flight only. The board owns the rest; duplicating
  it is what made the old nav feel heavy.
- Amber means an agent is working — filled running, hollow needs-you. Red only ever means a
  run ended badly. Blue is only ever a link out. Nothing else is saturated.
- Cards have no border and no background at rest; hover and focus are the only chrome.
- Every action has a key. If you add a button, add the key first and label the button with it.
