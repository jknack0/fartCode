# ADR log — ade

Lightweight Architecture Decision Records for **ade**. Every non-obvious
architectural choice a ticket makes gets a numbered ADR here — especially
*deviations* from ARCHITECTURE.md or the reference implementation (those are
the ones that bite later).

## When to write one

Write an ADR when a ticket:

- **Deviates from ARCHITECTURE.md or the reference** (trait placement, crate
  boundaries, algorithm semantics, schema choices) — mandatory;
- Introduces a new architectural pattern or changes an existing one;
- Resolves a genuinely ambiguous design question (pick one answer, record why).

Purely mechanical choices (which test helper, formatting) do **not** need an ADR.

## Format

One file per decision: `decisions/NNNN-slug.md`, where `NNNN` is the next
number (0001, 0002, …). Copy the template below. Keep it short — a few
sentences per section; the point is *why*, not prose.

```markdown
# ADR-NNNN: <Title>

- **Status:** Accepted | Proposed | Superseded by ADR-NNNN
- **Date:** YYYY-MM-DD
- **Ticket:** E1-XX
- **Relates to:** ARCHITECTURE.md §N.N / §18 D# (optional)

## Context
What problem, constraint, or ambiguity drove this decision.

## Decision
What we chose, and (if a deviation) what ARCHITECTURE/reference says instead.

## Consequences
What this makes easier, harder, or what later tickets must know.
```

## Rules

- **Accepted** = implemented (or implemented-with). Don't write an ADR before
  the code lands — decide, implement, then record.
- Numbering is append-only; never renumber or edit an Accepted ADR in place —
  supersede it.
- The ADR file is the durable note; ARCHITECTURE.md §18 keeps the scan-able
  index. When a decision changes, update the ADR (via supersession) *and* the
  index.
- Backfilled for E1-01…E1-03: 0001–0004 (see ARCHITECTURE.md §18 D1–D11).
