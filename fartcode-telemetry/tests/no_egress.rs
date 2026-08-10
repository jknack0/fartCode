//! The constraint this crate exists under: **no metric leaves the machine**
//! (ADR-0038 item 7 — "in-app only", "all local — no metric leaves the
//! machine").
//!
//! A comment saying so is worth nothing the day someone adds an opt-in
//! share. This is a static check, in the same spirit as
//! `fartcode-app/tests/no_blocking_tauri_commands.rs`: it reads this
//! crate's own manifest and sources as text — no network, no app instance —
//! and fails the build if either grows a way to talk to anything.
//!
//! Three rules, all about *capability* rather than intent:
//!
//! 1. The manifest declares **only** the dependencies on [`ALLOWED_DEPS`].
//!    An allowlist, not a denylist: a denylist passes by default, so it
//!    tests the imagination of whoever wrote it rather than the constraint
//!    — the next HTTP crate, or a wrapper around one, sails straight
//!    through a list of names nobody thought of. The workspace manifest
//!    still lists `reqwest` under "declared for integrations/**telemetry**";
//!    that reservation predates ADR-0038 item 7, and this rule is what
//!    voids it for this crate.
//! 2. No source file names a network or subprocess API — the routes that
//!    need no new dependency at all (`std::net`, or shelling out).
//! 3. No source file performs I/O of any kind. That is the positive form
//!    of the same promise: the app layer gathers, this crate computes, so
//!    there is exactly one place where "local" has to be true.
//!
//! It deliberately does not inspect transitive dependencies. `serde` could
//! in principle pull in the world; what matters is that nothing *here* can
//! call it. Keeping this crate's own dependency list to two entries is what
//! makes the claim verifiable by eye as well as by test.
//!
//! Comment lines are stripped before matching. The modules genuinely need
//! to name `fartcode_core::dossiers` in their docs to say where their input
//! comes from, and a guard that trips on its own explanations only teaches
//! people to delete the explanations.

use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every dependency this crate may declare, in any dependency table.
///
/// **Adding an entry is the reviewable act.** It is a claim that the crate
/// still cannot reach anything off this machine, and it belongs in the same
/// change as the ADR that allows it.
const ALLOWED_DEPS: &[&str] = &["serde", "serde_json"];

/// Source substrings that would mean egress with no new dependency.
///
/// Not listed: `http://` and `https://`. A URL in a test fixture (a dossier
/// timeline's `pr merged · https://…` breadcrumb) is a string, not a
/// capability, and forbidding the characters would only push fixtures into
/// being unrealistic.
const FORBIDDEN_APIS: &[&str] = &[
    "std::net",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "process::Command",
];

/// APIs that would mean this crate stopped being pure.
const FORBIDDEN_IO: &[&str] = &["std::fs", "File::open", "rusqlite", "fartcode_core::"];

/// Every `.rs` file in the crate, comments stripped, minus this file.
fn code() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(&crate_root().join("src"), &mut paths);
    walk(&crate_root().join("tests"), &mut paths);

    let out: Vec<(PathBuf, String)> = paths
        .into_iter()
        .filter(|p| !p.ends_with("no_egress.rs"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let code: String = text
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            Some((path, code))
        })
        .collect();

    // Rule 4, implicit: the guard must fail loudly rather than go blind.
    assert!(
        out.len() >= 6,
        "expected the whole crate to be scanned, found {} file(s)",
        out.len()
    );
    out
}

/// Dependency-table keys declared in this crate's manifest.
///
/// A deliberately small hand parser rather than a TOML crate: adding a
/// dependency to prove the crate has none would be its own joke, and the
/// manifest's shape is fixed and one screen long. It reads every
/// `[...dependencies...]` table (including `[target.'...'.dependencies]`)
/// and takes the key left of the first `=`.
fn declared_deps(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_deps = line.trim_end_matches(']').ends_with("dependencies");
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() {
                out.push(key.to_string());
            }
        }
    }
    out
}

#[test]
fn the_manifest_declares_only_allowlisted_dependencies() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml")).unwrap();
    let declared = declared_deps(&manifest);

    // The guard must see something, or it is passing on an empty parse.
    assert!(
        declared.contains(&"serde".to_string()),
        "parsed no dependency tables from the manifest — the guard went blind: {declared:?}"
    );

    for dep in &declared {
        assert!(
            ALLOWED_DEPS.contains(&dep.as_str()),
            "fartcode-telemetry declares `{dep}`, which is not on this test's allowlist \
             ({ALLOWED_DEPS:?}). ADR-0038 item 7: no metric leaves the machine, and this \
             crate performs no I/O at all. If a signal genuinely needs a new dependency, \
             add it here in the same change — and say why."
        );
    }
}

/// The allowlist is only as good as its ability to reject.
#[test]
fn the_dependency_parser_sees_a_new_entry() {
    let manifest = "\
[package]
name = \"fartcode-telemetry\"

[dependencies]
serde = { workspace = true }
reqwest = \"0.12\"

[dev-dependencies]
serde_json = { workspace = true }

[target.'cfg(unix)'.dependencies]
libc = \"0.2\"
";
    let declared = declared_deps(manifest);
    assert_eq!(declared, ["serde", "reqwest", "serde_json", "libc"]);
    assert!(declared.iter().any(|d| !ALLOWED_DEPS.contains(&d.as_str())));
}

#[test]
fn no_source_can_open_a_connection_or_spawn_a_process() {
    for (path, code) in code() {
        for api in FORBIDDEN_APIS {
            assert!(
                !code.contains(api),
                "{} names `{api}` — fartcode-telemetry must have no way to reach anything \
                 off this machine (ADR-0038 item 7)",
                path.display()
            );
        }
    }
}

#[test]
fn no_source_performs_io() {
    for (path, code) in code() {
        for api in FORBIDDEN_IO {
            assert!(
                !code.contains(api),
                "{} names `{api}` — fartcode-telemetry performs no I/O; the app layer \
                 gathers, this crate computes",
                path.display()
            );
        }
    }
}
