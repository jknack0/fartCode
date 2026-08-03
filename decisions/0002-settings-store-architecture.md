# ADR-0002: Settings store architecture (object-safe trait, delta semantics)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E1-02, E1-03
- **Relates to:** ARCHITECTURE.md §6.2 / §18 D4–D7

## Context

ARCHITECTURE §6.2 sketched `SettingsStore` with generic methods
(`fn get<T: SettingValue>(&self, key: SettingKey<T>)`), but §7 wires it as
`Arc<dyn SettingsStore>` — generic methods are not object-safe, so the sketch
as written cannot compile.

## Decision

- **Deviation (§6.2):** the trait is **object-safe** with a JSON surface
  (`get_json`/`set_json`/`reset`/`share_with_team` + the project-settings
  methods `projects` needs). Typed access (`settings::get(&PROJECT)`) lives on
  the concrete `DbSettingsStore` via `SettingKey<T>` wrappers.
- **App settings are delta-vs-defaults:** `set` computes the delta vs the
  registry defaults; an empty delta **deletes the row** (updating to the
  default = reset). Reads deep-merge the stored delta with defaults. Values are
  validated by canonical round-trip (unknown keys stripped — zod-parse
  behavior), so deltas never carry junk.
- **Effective project-settings precedence:** `defaults < .ade.json <
  DB-shareable` (later source wins — the reference's
  `mergeShareableProjectSettings(defaults, file, local)`). A local UI value
  overrides the file; clearing it falls back to the file; clearing the file
  falls back to defaults.
- `update_project_settings` is **full-replace** (reference `update()`); callers
  read-modify-write. The repo `.ade.json` is only touched by `share_with_team`.
- Legacy `.emdash.json` migration is a **one-shot at first access** (marked
  done even without a file; a single marker covers base+shareable). Shareable
  merge is unconditional — the reference gates it on git-tracking, which needs
  `ade-git`.

## Consequences

- The trait works as a trait object (§7 wiring holds) at the cost of JSON
  round-trips at the boundary.
- "Set to default deletes the row" makes defaults observable and resets trivial.
- Full-replace update is a footgun for partial edits — documented on the method;
  a `patch`-style API may be needed when the settings UI lands (E1-05).
- The `.ade.json` file is the shareable contract with teammates.
