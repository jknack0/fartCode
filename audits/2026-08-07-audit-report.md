# Prioritized Audit Report — fartCode

**Date:** 2026-08-07 · **Scope:** `fartcode-core` + `fartcode-app` (+ `fartcode-git` where noted) · **Base commit:** `d44139e`
**Inputs:** 4 audit scans (unwrap/expect/panic, unsafe, TODO/FIXME, debug leftovers) + cross-validation report (`store: validation-report`).

**Verdict: no false positives in any audit output, no missed issues.** Every finding was
line-by-line verified against source, and I independently re-ran each scan. Net result:
**2 confirmed fixes (both low-severity, currently-unreachable-in-practice defensive items)
+ 1 optional lint hardening.** Everything else is correctly classified benign/safe.

No critical or high-severity issues exist in the audited surface.

---

## Recommended actions, in priority order

| # | Priority | Where | Action | Why now | Cost |
|---|----------|-------|--------|---------|------|
| 1 | **P1** | `fartcode-core/src/settings/service.rs:757` | Replace `expect("config is guaranteed an object")` with `if let Some(obj) = config.as_object_mut() { … }` | Invariant is caller-enforced only; one future call site turns user-editable `.fartCode.json` into a panic. Currently unreachable (single caller normalizes at 565–567). | ~3 lines |
| 2 | **P1** | `fartcode-core/src/fs_watch/mod.rs:291` | Map thread-spawn failure to `Error::Watch` or log-and-degrade instead of `.expect(...)` | A spawn panic inside `pub fn FsWatchService::new(...) -> Result<Self, Error>` defeats the cross-crate Result contract ("no panics across crate boundaries" per AGENTS.md). Fails only on resource exhaustion. | ~5 lines |
| 3 | **P3** (optional) | both `lib.rs` | `#![forbid(unsafe_code)]` | De-facto → de-jure hardening. Zero behavior change, aligns with merge gate. Not required. | 1 line × 2 |

**P2 (tracked, do not fix now):** `fartcode-core/src/tasks/operations.rs:283` — the only
TODO in non-test source: `workspace_id` FK TOCTOU (plain TEXT column, schema hash-verified).
Deliberate, documented deferral owned by E2-05 (append-only migration work). Leave in place;
keep the ticket attached.

---

## Audit 1 — unwrap / expect / panic: CONFIRMED, enumeration complete

Cross-validation reproduced the production count with an independent brace-aware
`#[cfg(test)]`/`#[test]` stripper: **exactly 11 production sites**, all accounted for.
Zero misses. My own check confirmed the two actionable sites and the test-gating of every
panic macro.

**Finding 1 — REAL (P1).** `apply_shareable` (private fn, line 754) has exactly one caller
(line 568) that normalizes non-object roots at 565–567. The `expect` at 757 is
caller-enforced only. Verified in source: caller path is
`if !config.is_object() { config = Value::Object(Default::default()) }` → `apply_shareable`.
Fix: `if let Some(obj) = config.as_object_mut()`.

**Finding 2 — REAL (P1).** `spawn_dispatcher` (line 261) is called from
`pub fn FsWatchService::new` (line 100), which returns `Result<Self, Error>` (cross-crate).
A thread-spawn failure panics inside a Result-returning public constructor. Verified in
source: `.expect("spawning fs-watch dispatcher thread")` at 291.

**Findings 3–4 — benign, confirmed:** `(0..).find().expect("u32 slot space")` fires only with
2³² live tmux sessions (correct); `LazyLock` over `include_str!` embedded JSON is
compile-time bundled and CI-validated (re-init after panic, no poison — note is correct).

**Findings 5–9 — provably safe, confirmed:** (5) `Stdio::piped` post-spawn handle guarantee;
(6) `fs_watch:170` just-acquired under held registry lock; (7) `fs_watch:215` get_mut-checked
above, same lock, no intervening removal; (8) `pty/mod.rs:120–121` guarded by `is_some()` on
immutable `&ProviderDescriptor`/`&CommandContext` borrows; (9) `resource_monitor.rs:33`
just-populated under held MutexGuard.

**"0 panic macros in production" — confirmed.** All panic-family macros live in
`#[cfg(test)]` mods. Independent count: **6 sites across 2 crates**, all test-gated:
`fs_watch/mod.rs:491` (mod tests @408), `events.rs:225` (@206), `secrets.rs:98` (@67),
`provider_accounts/mod.rs:469` + `533` (@329), `fartcode-git/status.rs:374`
(`#[cfg(test)] mod tests` @342). `fartcode-git/lib.rs:508` = `#[cfg(test)]` / 509 =
`mod tests {`; the only hits above it are non-panicking `unwrap_or` (245) /
`unwrap_or_default` (395). *(Validation label said "5 hits" but listed 6 sites — trivial
label slip; substance confirmed.)*

## Audit 2 — unsafe: CONFIRMED clean

Independent scan: 0 `unsafe` keywords, 0 transmute/MaybeUninit/from_raw, 0 raw pointer
casts, 0 `extern "C"`/FFI, 0 `asm!` in non-test source. Whole-workspace re-check: the only
`unsafe` substring is the provider CLI flag string `"--skip-permissions-unsafe"` in
`providers_data.rs:1521` — not code. `git2`/`rusqlite`/`notify`/`portable-pty` use unsafe
internally, but only their safe APIs are called; CLI git goes through `Command` with arg
arrays (no shell-injection surface). The `#![forbid(unsafe_code)]` recommendation (P3) is
sound optional hardening.

## Audit 3 — TODO/FIXME: CONFIRMED

Only marker in non-test source: `tasks/operations.rs:283` (workspace_id FK TOCTOU,
owner-tracked to E2-05 — see P2 above). No FIXMEs, no ponytail markers. Exclusions verified:
`Todo` enum variant (`model.rs`), `refs/heads/xxx` doc comment (`git.rs:38`).

## Audit 4 — debug leftovers: CONFIRMED clean

0 `println!`/`dbg!`/`eprintln!`/`print!` in non-test src of both crates.
`examples/smoke.rs` has exactly 13 `println!` — deliberate, documented checkpoint output for
the manual E1-02 example (tempfile-based, `cargo run -p fartcode-core --example smoke`).
Not shipped surface; no action.

---

## Staleness check

Audited files last changed by `d44139e` (rename). 6 files in the audited crates have
working-tree edits (`db/migrations.rs`, `provider_accounts/mod.rs`, `pty/launcher.rs`,
`pty/mod.rs`, `terminals/lifecycle.rs`, `terminals/pty.rs`); diff review shows all newly
added unwraps/expects (incl. "default account exists") land inside `#[cfg(test)]` mods.
**Findings hold against the current working tree.**

## Merge-gate impact

- Fixes #1 and #2 each satisfy the existing gate (`cargo fmt`, `clippy -D warnings`,
  `cargo test`) with no new deps, no migration, no schema change — both can ride any Phase 0
  ticket as drive-by hardening.
- Fix #2's error-vs-degrade choice is a small design decision: prefer `Error::Watch` (keeps
  the constructor's Result contract honest); log-and-degrade only if callers must stay
  resilient to a dead watcher. Record as an ADR only if the degrade path is chosen.
- No ticket needed for #3; it is a one-line lint, safe to fold into the next merge.

## Not-a-bug notes (do not re-audit)

- `unwrap_or*` at `fartcode-git/lib.rs:245/395` — non-panicking by design.
- Panic macros in test code — intentional, exercise the panic paths.
- `smoke.rs` printlns — deliberate example checkpoint output.
