//! Regression guard for #80 — "93 of 101 tauri commands block the macOS main
//! thread".
//!
//! A non-`async` `#[tauri::command]` compiles to `ExecutionContext::Blocking`
//! (tauri-macros `command/wrapper.rs` → `body_blocking`): the body is inlined
//! into the invoke handler and resolved synchronously. The invoke handler runs
//! inline on the IPC thread, and on macOS that thread IS the main thread — so
//! any blocking work there stalls the NSRunLoop and the window stops
//! repainting. A beachball, not a spinner.
//!
//! This test is a **static** check: it parses `src/lib.rs`'s `generate_handler!`
//! list and the `#[tauri::command]` functions in `src/commands/*.rs` as text.
//! No app instance, no window, no DB, no GUI — it runs anywhere `cargo test`
//! runs, which is how it reaches CI (`cargo test --workspace`) and `make check`
//! without a workflow edit.
//!
//! **The contract it enforces**
//!
//! 1. Every registered command is either `async`, or listed in [`SYNC_OK`] with
//!    a justification. A NEW command defaults to failing — the author has to
//!    make a deliberate, reviewable choice.
//! 2. Every command in [`MUST_OFFLOAD`] (#80's confirmed offender table) is
//!    `async` AND hands its blocking body to `spawn_blocking`. `async` alone
//!    only moves the block onto an async-runtime worker; the work must leave
//!    the thread.
//! 3. No `SYNC_OK` command's body contains an obvious blocking call
//!    (subprocess spawn, thread sleep, blocking HTTP) — that is how an
//!    allowlisted command silently rots into a freeze.
//! 4. Neither list carries a stale entry, and every registered command is
//!    findable. The guard fails loudly rather than going quietly blind.
//!
//! See AGENTS.md § "Tauri commands and the main thread".

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Commands that may stay non-`async`: the whole call path is cheap and
/// bounded — a pure function, an in-memory lookup, a short SQLite statement,
/// or a single small file read/write. **Adding an entry is a claim that you
/// walked the entire call path**, not just the command body.
///
/// Not permitted here: anything that reaches a subprocess, the network, a
/// sleep, a process spawn, or unbounded filesystem work.
#[rustfmt::skip]
const SYNC_OK: &[(&str, &str)] = &[
    // -- projects / tasks (SQLite only) -----------------------------------
    ("list_projects", "SELECT over projects"),
    ("list_tasks", "SELECT over tasks"),
    ("toggle_pin", "single-row read + UPDATE"),
    // -- issues board (SQLite only; ARCHITECTURE §13) ---------------------
    ("issue_create", "INSERT + lane position bookkeeping"),
    ("issue_list", "SELECT + in-memory blocked-status derivation"),
    ("issue_update", "single-row UPDATE"),
    ("issue_move", "lane/position UPDATE"),
    ("issue_delete", "single-row DELETE"),
    ("issue_link", "dependency INSERT + cycle check"),
    ("issue_unlink", "dependency DELETE"),
    ("issue_import_github", "pure field mapping + INSERT; the gh fetch already happened"),
    ("issue_parse_proposal", "pure parser over a string"),
    ("issue_apply_proposal", "one SQLite transaction"),
    // -- line comments (SQLite only; ARCHITECTURE §14) --------------------
    ("add_line_comment", "INSERT + event emit"),
    ("agent_add_line_comment", "validation + INSERT"),
    ("list_line_comments", "SELECT over line_comments"),
    ("resolve_line_comment", "single-row UPDATE"),
    ("delete_line_comment", "single-row DELETE"),
    // -- conversations (SQLite / in-memory) -------------------------------
    ("create_conversation", "provider lookup + INSERT"),
    ("list_conversations", "SELECT over conversations"),
    ("list_project_conversations", "SELECT + in-memory filter"),
    ("get_or_create_project_conversation", "SELECT then optional INSERT"),
    ("acp_resolve_permission", "in-memory channel send to the live session"),
    ("acp_stop", "drops the session record; adapter shutdown is already spawned"),
    ("acp_history", "in-memory LiveModels snapshot"),
    // -- provider accounts (SQLite + keyring) -----------------------------
    // Keyring access is an IPC to the OS credential store, not a subprocess.
    // It is bounded in practice; revisit if a keychain access prompt is ever
    // observed to stall these.
    ("add_provider_account", "keyring store + INSERT"),
    ("list_provider_accounts", "SELECT + keyring reads for the display mask"),
    ("remove_provider_account", "DELETE + keyring delete"),
    ("set_default_provider_account", "single-row UPDATE"),
    ("list_providers", "static provider registry, no I/O"),
    // -- github token (keyring only; the gh CLI path is github_token_import)
    ("github_token_status", "keyring read + mask"),
    ("github_token_set", "keyring write"),
    ("github_token_clear", "keyring delete"),
    // -- terminals (in-memory / PTY ioctl) --------------------------------
    ("terminal_write", "writes bytes to an open PTY handle"),
    ("terminal_resize", "PTY resize ioctl"),
    ("terminal_list_for_task", "in-memory terminal map"),
    ("terminal_tail", "in-memory scrollback buffer"),
    // -- settings / view state / search -----------------------------------
    ("get_project_settings", "SQLite + one .fartCode.json read"),
    ("update_project_settings", "path validation + SQLite + one .fartCode.json read"),
    ("share_with_team", "SQLite + one .fartCode.json patch"),
    ("get_view_state", "kv SELECT"),
    ("set_view_state", "kv UPSERT"),
    ("search", "SQLite FTS query"),
    ("resource_sample", "in-process sysinfo refresh"),
    ("get_resource_monitor_enabled", "settings SELECT"),
    ("set_resource_monitor_enabled", "settings UPSERT"),
    // -- files -------------------------------------------------------------
    ("write_workspace_file", "one contained write inside the worktree"),
];

/// #80's confirmed offenders: commands whose call path is *unbounded* blocking
/// work. These must be `async` **and** must push the blocking body off the
/// thread with `spawn_blocking` — an `async fn` with a synchronous body just
/// relocates the stall onto an async-runtime worker.
///
/// Entries never move to [`SYNC_OK`]. If a command here stops doing blocking
/// work entirely, delete it from the list along with the work.
#[rustfmt::skip]
const MUST_OFFLOAD: &[(&str, &str)] = &[
    ("create_task", "create_with_provision → git fetch + git worktree add (no bound: a cold remote freezes the UI)"),
    ("delete_task", "teardown_sessions → 5s TEARDOWN_WAIT spin-sleep per leaf, tmux kills, git worktree remove"),
    ("git_push", "fartcode_git::remote → git push over the network with no timeout"),
    ("git_create_pr", "pushes when unpublished, then resolves the compare URL — network"),
    ("git_fetch", "git fetch over the network with no timeout"),
    ("git_pull", "git pull --ff-only over the network with no timeout"),
    ("terminal_open", "tmux slot probe (list_tmux_sessions_by_prefix) + PTY spawn"),
    ("provider_auth_status", "CLI auth probe subprocess, bounded at 5s — five seconds of frozen window"),
];

/// Calls that are never acceptable inside a non-`async` command body. Each is
/// unambiguous: seeing one in a command body means that command spawns a
/// process, sleeps, or blocks on the network on the main thread.
#[rustfmt::skip]
const BLOCKING_MARKERS: &[(&str, &str)] = &[
    ("std::process::Command", "spawns a subprocess"),
    ("Command::new(", "spawns a subprocess"),
    ("thread::sleep", "sleeps the calling thread"),
    ("reqwest::blocking", "makes a blocking HTTP request"),
    ("ureq::", "makes a blocking HTTP request"),
];

// ── the guard ────────────────────────────────────────────────────────────────

#[test]
fn no_tauri_command_blocks_the_main_thread() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registered = parse_invoke_handler(&crate_root.join("src/lib.rs"));
    assert!(
        registered.len() > 50,
        "{}",
        blind_guard_message(&format!(
            "only {} commands parsed out of src/lib.rs's generate_handler! list",
            registered.len()
        ))
    );

    let sync_ok: BTreeSet<&str> = SYNC_OK.iter().map(|(name, _)| *name).collect();
    let must_offload: BTreeSet<&str> = MUST_OFFLOAD.iter().map(|(name, _)| *name).collect();
    let mut violations: Vec<String> = Vec::new();

    // Rule 4a — the two lists must not disagree about the same command.
    for name in sync_ok.intersection(&must_offload) {
        violations.push(format!(
            "`{name}` is in BOTH SYNC_OK and MUST_OFFLOAD in {GUARD}.\n\
             MUST_OFFLOAD wins by construction — a command doing unbounded blocking work\n\
             can never be allowed to stay synchronous. Remove the SYNC_OK entry."
        ));
    }

    // Rule 4b — a list entry that names nothing real has stopped guarding
    // anything. Stale entries are how this file rots.
    let registered_names: BTreeSet<&str> = registered.iter().map(|c| c.name.as_str()).collect();
    for (list_name, entries) in [("SYNC_OK", &sync_ok), ("MUST_OFFLOAD", &must_offload)] {
        for name in entries.difference(&registered_names) {
            violations.push(format!(
                "{list_name} in {GUARD} lists `{name}`, which is not registered in\n\
                 fartcode-app/src/lib.rs's invoke_handler. Remove the stale entry — these\n\
                 lists must describe the live command surface or they stop guarding it."
            ));
        }
    }

    // Parse every command module once.
    let mut modules: BTreeMap<String, BTreeMap<String, CommandFn>> = BTreeMap::new();
    for command in &registered {
        modules.entry(command.module.clone()).or_insert_with(|| {
            let path = crate_root
                .join("src/commands")
                .join(format!("{}.rs", command.module));
            parse_command_fns(&path)
        });
    }

    for entry in &registered {
        let Some(func) = modules[&entry.module].get(&entry.name) else {
            // Rule 4c — the parser lost the command. Fail loudly; a guard that
            // silently skips what it cannot classify guards nothing.
            violations.push(blind_guard_message(&format!(
                "registered command `commands::{}::{}` has no #[tauri::command] fn in \
                 fartcode-app/src/commands/{}.rs",
                entry.module, entry.name, entry.module
            )));
            continue;
        };
        let site = format!(
            "fartcode-app/src/commands/{}.rs:{}",
            entry.module, func.line
        );

        if must_offload.contains(entry.name.as_str()) {
            // Rule 2 — async AND actually off the thread.
            let why = MUST_OFFLOAD
                .iter()
                .find(|(n, _)| *n == entry.name)
                .map(|(_, why)| *why)
                .unwrap_or("");
            if !func.is_async {
                violations.push(must_offload_message(&entry.name, &site, why, false));
            } else if !func.code.contains("spawn_blocking") {
                violations.push(must_offload_message(&entry.name, &site, why, true));
            }
            continue;
        }

        if !func.is_async && !sync_ok.contains(entry.name.as_str()) {
            // Rule 1 — the default-deny gate that catches NEW commands.
            violations.push(unclassified_message(&entry.name, &site));
            continue;
        }

        if !func.is_async {
            // Rule 3 — an allowlisted command that grew a blocking call.
            if let Some((marker, what)) = BLOCKING_MARKERS
                .iter()
                .find(|(marker, _)| func.code.contains(marker))
            {
                violations.push(marker_message(&entry.name, &site, marker, what));
            }
        }
    }

    if !violations.is_empty() {
        let body = violations
            .iter()
            .enumerate()
            .map(|(i, v)| format!("── {}/{} ──\n{v}\n", i + 1, violations.len()))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "\n\n{} tauri command(s) violate the #80 main-thread rule.\n\n{body}\n\
             Rule: AGENTS.md § \"Tauri commands and the main thread\".\n",
            violations.len()
        );
    }
}

// ── failure messages (the whole point of the guard) ──────────────────────────

const GUARD: &str = "fartcode-app/tests/no_blocking_tauri_commands.rs";

const AWAIT_RULE: &str = "\
Never hold the DB connection guard (`app.db.conn().lock()`) across an `.await`:\n\
the connection mutex is non-reentrant, so a re-entrant lock deadlocks the app.\n\
Scope the guard inside the spawn_blocking closure.";

fn unclassified_message(name: &str, site: &str) -> String {
    format!(
        "`{name}` ({site}) is registered as a NON-ASYNC #[tauri::command].\n\
         \n\
         A non-async command runs inline on the IPC thread — the macOS MAIN thread.\n\
         Blocking there freezes the window (beachball, not spinner). See #80.\n\
         \n\
         Pick one:\n\
         \n\
         (a) It touches a subprocess, the network, a sleep, a process spawn, or\n\
         \x20    unbounded filesystem work — make it async and push the blocking body\n\
         \x20    off the thread:\n\
         \n\
         \x20        #[tauri::command]\n\
         \x20        pub async fn {name}(app: State<'_, Arc<App>>, ..) -> Result<T, String> {{\n\
         \x20            let app = app.inner().clone();\n\
         \x20            tauri::async_runtime::spawn_blocking(move || {{\n\
         \x20                /* the blocking body, verbatim */\n\
         \x20            }})\n\
         \x20            .await\n\
         \x20            .map_err(|e| e.to_string())?\n\
         \x20        }}\n\
         \n\
         (b) It is genuinely cheap and bounded (pure fn, in-memory, one short SQLite\n\
         \x20    statement) — add `(\"{name}\", \"<why>\")` to SYNC_OK in\n\
         \x20    {GUARD}.\n\
         \x20    Adding an entry is a claim that you walked the WHOLE call path, not\n\
         \x20    just the command body.\n\
         \n\
         {AWAIT_RULE}"
    )
}

fn must_offload_message(name: &str, site: &str, why: &str, is_async: bool) -> String {
    let head = if is_async {
        format!(
            "`{name}` ({site}) is async but its body never calls `spawn_blocking`.\n\
             \n\
             Making a command async only moves the stall onto an async-runtime worker.\n\
             The blocking work must LEAVE the thread."
        )
    } else {
        format!(
            "`{name}` ({site}) does unbounded blocking work but is NOT async.\n\
             \n\
             A non-async command runs inline on the macOS main thread — this one freezes\n\
             the window for as long as the work takes."
        )
    };
    format!(
        "{head}\n\
         \n\
         Blocking work on this path (#80): {why}\n\
         \n\
         Required shape — async + tauri::async_runtime::spawn_blocking:\n\
         \n\
         \x20    #[tauri::command]\n\
         \x20    pub async fn {name}(app: State<'_, Arc<App>>, ..) -> Result<T, String> {{\n\
         \x20        let app = app.inner().clone();\n\
         \x20        tauri::async_runtime::spawn_blocking(move || {{\n\
         \x20            /* the blocking body, verbatim */\n\
         \x20        }})\n\
         \x20        .await\n\
         \x20        .map_err(|e| e.to_string())?\n\
         \x20    }}\n\
         \n\
         This command is in MUST_OFFLOAD in {GUARD}; it may not be moved to SYNC_OK.\n\
         \n\
         {AWAIT_RULE}"
    )
}

fn marker_message(name: &str, site: &str, marker: &str, what: &str) -> String {
    format!(
        "`{name}` ({site}) is allowlisted in SYNC_OK but its body contains `{marker}` —\n\
         it {what} on the macOS main thread. See #80.\n\
         \n\
         Make it `async` and wrap the blocking body in\n\
         `tauri::async_runtime::spawn_blocking(..).await`, then delete its SYNC_OK entry.\n\
         \n\
         {AWAIT_RULE}"
    )
}

fn blind_guard_message(detail: &str) -> String {
    format!(
        "the #80 guard went blind: {detail}.\n\
         \n\
         {GUARD} classifies commands by parsing source text. It just failed to see the\n\
         command surface, so it is no longer guarding anything. Fix the parser (or the\n\
         thing that moved) rather than deleting the check."
    )
}

// ── parsing (plain text; no syn dependency) ──────────────────────────────────

/// One `commands::<module>::<name>` entry of the `generate_handler!` list.
struct Registered {
    module: String,
    name: String,
}

/// A parsed `#[tauri::command]` function: whether it is `async`, and its body
/// with comments and string contents stripped so marker matching can't be
/// fooled by a doc comment.
struct CommandFn {
    is_async: bool,
    line: usize,
    code: String,
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{}",
            blind_guard_message(&format!("{}: {e}", path.display()))
        )
    })
}

/// Pulls the `commands::<module>::<name>` paths out of
/// `invoke_handler(tauri::generate_handler![ .. ])`.
fn parse_invoke_handler(lib_rs: &Path) -> Vec<Registered> {
    let src = read(lib_rs);
    let start = src.find("generate_handler![").unwrap_or_else(|| {
        panic!(
            "{}",
            blind_guard_message("no `generate_handler![` in fartcode-app/src/lib.rs")
        )
    });
    let rest = &src[start..];
    let end = rest.find(']').unwrap_or_else(|| {
        panic!(
            "{}",
            blind_guard_message("unterminated `generate_handler![` list in src/lib.rs")
        )
    });

    rest[..end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',').trim();
            if line.is_empty() || line.starts_with("//") {
                return None;
            }
            let mut parts = line.rsplitn(3, "::");
            let name = parts.next()?.trim().to_string();
            let module = parts.next()?.trim().to_string();
            Some(Registered { module, name })
        })
        .collect()
}

/// Maps command name → parsed fn for one `src/commands/<module>.rs`.
fn parse_command_fns(path: &Path) -> BTreeMap<String, CommandFn> {
    parse_command_fns_src(&read(path))
}

fn parse_command_fns_src(src: &str) -> BTreeMap<String, CommandFn> {
    let mut out = BTreeMap::new();
    let lines: Vec<&str> = src.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        // Skip further attributes / doc comments between the attribute and
        // the signature.
        let Some(sig_idx) = (i + 1..lines.len()).find(|&j| {
            let t = lines[j].trim_start();
            t.starts_with("pub fn ") || t.starts_with("pub async fn ")
        }) else {
            continue;
        };
        let sig = lines[sig_idx].trim_start();
        let is_async = sig.starts_with("pub async fn ");
        let after_fn = sig.trim_start_matches("pub ").trim_start_matches("async ");
        let name = after_fn
            .trim_start_matches("fn ")
            .split(['(', '<'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        // Rejoin from the signature rather than indexing by byte offset —
        // correct regardless of line endings.
        let code = strip_to_code(&lines[sig_idx..].join("\n"));
        out.insert(
            name,
            CommandFn {
                is_async,
                line: sig_idx + 1,
                code,
            },
        );
    }
    out
}

/// Returns the function body that starts at the first `{` of `src`, with
/// comments and string-literal contents removed, so `spawn_blocking` and the
/// blocking markers only ever match real code.
///
/// Char literals are deliberately not tracked: Rust lifetimes (`State<'_, T>`)
/// would defeat a naive `'` scanner, and no command body puts a brace in a
/// char literal.
fn strip_to_code(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut i = match src.find('{') {
        Some(i) => i,
        None => return String::new(),
    };
    let mut depth = 0usize;
    let mut out = String::new();
    while i < bytes.len() {
        let rest = &src[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i += rest.find("*/").map(|p| p + 2).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("r#\"") || rest.starts_with("r\"") {
            let (open, close) = if rest.starts_with("r#\"") {
                ("r#\"", "\"#")
            } else {
                ("r\"", "\"")
            };
            let after = &rest[open.len()..];
            i += open.len()
                + after
                    .find(close)
                    .map(|p| p + close.len())
                    .unwrap_or(after.len());
            continue;
        }
        let c = bytes[i] as char;
        if c == '"' {
            i += 1;
            while i < bytes.len() {
                match bytes[i] as char {
                    '\\' => i += 2,
                    '"' => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            continue;
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

// ── the guard's own tests: a parser that quietly breaks guards nothing ───────

/// Exercises every branch the classifier depends on: async detection, an
/// attribute/doc-comment gap between `#[tauri::command]` and the signature,
/// nested braces in the body, and comment/string stripping (a blocking call
/// *named* in a comment or a string literal must not count as one).
#[test]
fn parser_classifies_async_and_ignores_comments_and_strings() {
    let fixture = r#"
//! module docs

#[tauri::command]
pub fn cheap(app: State<'_, Arc<App>>) -> Result<(), String> {
    // std::process::Command::new("git") — named in a comment only
    let label = "thread::sleep";
    Ok(drop((app, label)))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
/// Doc comment between the attribute and the signature.
pub async fn heavy(app: State<'_, Arc<App>>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || { drop(app); })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn spawns() -> Result<(), String> {
    let _ = std::process::Command::new("tmux").status();
    Ok(())
}
"#;
    let parsed = parse_command_fns_src(fixture);
    assert_eq!(
        parsed.keys().collect::<Vec<_>>(),
        vec!["cheap", "heavy", "spawns"],
        "the parser must find every #[tauri::command] fn"
    );

    let cheap = &parsed["cheap"];
    assert!(!cheap.is_async);
    for (marker, _) in BLOCKING_MARKERS {
        assert!(
            !cheap.code.contains(marker),
            "`{marker}` inside a comment or string literal must not count as a blocking call"
        );
    }

    let heavy = &parsed["heavy"];
    assert!(heavy.is_async, "`pub async fn` must be seen as async");
    assert!(
        heavy.code.contains("spawn_blocking"),
        "the body scan must survive nested braces and an attribute/doc gap"
    );

    let spawns = &parsed["spawns"];
    assert!(!spawns.is_async);
    assert!(
        BLOCKING_MARKERS
            .iter()
            .any(|(marker, _)| spawns.code.contains(marker)),
        "a real subprocess spawn in a sync command body must be detected"
    );
}

/// The handler-list parser must see the real command surface — this is the
/// input everything else keys off.
#[test]
fn invoke_handler_parse_matches_the_registered_surface() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registered = parse_invoke_handler(&crate_root.join("src/lib.rs"));
    let names: BTreeSet<&str> = registered.iter().map(|c| c.name.as_str()).collect();

    assert_eq!(
        names.len(),
        registered.len(),
        "duplicate command names in the invoke_handler list"
    );
    // Spot-check both ends of the list and one mid-list module so a silent
    // truncation is caught.
    for expected in ["list_projects", "git_push", "terminal_open", "search"] {
        assert!(
            names.contains(expected),
            "{}",
            blind_guard_message(&format!(
                "`{expected}` missing from the parsed handler list"
            ))
        );
    }
    for entry in &registered {
        assert!(
            !entry.module.is_empty() && !entry.name.is_empty(),
            "malformed handler entry: {}::{}",
            entry.module,
            entry.name
        );
    }
}
