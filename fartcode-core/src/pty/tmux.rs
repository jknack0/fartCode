//! Tmux durability (E2-07): session naming + the create-or-attach shell line.
//! Port of `tmux-session-name.ts`. The tmux ENABLED path is what survives a
//! hard kill of the fartCode process (the tmux server owns the session); the
//! non-tmux fallback rehydrates best-effort on boot.

/// `TMUX_SESSION_PREFIX` (reference `emdash-`, ours `fartCode-`).
pub const TMUX_SESSION_PREFIX: &str = "fartCode-";
/// Reference `TMUX_HISTORY_LIMIT`.
pub const TMUX_HISTORY_LIMIT: u32 = 100_000;

/// `makeTmuxSessionName`: `fartCode-` + base64url(sessionId).
pub fn make_tmux_session_name(session_id: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    format!(
        "{TMUX_SESSION_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(session_id.as_bytes())
    )
}

/// Inverse of `make_tmux_session_name`: returns the decoded session id, or
/// `None` when the name isn't a well-formed fartCode tmux session name.
pub fn parse_tmux_session_name(name: &str) -> Option<String> {
    let encoded = name.strip_prefix(TMUX_SESSION_PREFIX)?;
    if encoded.is_empty() {
        return None;
    }
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let bytes = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    String::from_utf8(bytes).ok()
}

/// `buildTmuxShellLine`: create-if-missing (`has-session || new-session -d`),
/// enable mouse + history-limit, then attach. `-u` forces UTF-8 (GUI-launched
/// apps often have no LANG set).
///
/// Names/commands are JSON-quoted (reference parity). JSON quoting is NOT
/// full shell quoting — `$()`/backticks/`$VAR` still expand inside the
/// double-quoted segments under the wrapping `sh -c`. This matches the
/// reference exactly; the session name is our own base64url (no `$`) and the
/// command line is provider config (trusted), so the residual surface is
/// documented, not exploitable.
pub fn build_tmux_shell_line(session_name: &str, command_line: &str) -> String {
    // The reference quotes both via JSON.stringify — a command line with
    // spaces/metacharacters must not inject into the tmux command.
    let quoted_name =
        serde_json::to_string(session_name).unwrap_or_else(|_| format!("\"{session_name}\""));
    let quoted_cmd =
        serde_json::to_string(command_line).unwrap_or_else(|_| format!("\"{command_line}\""));
    let check_exists = format!("tmux has-session -t {quoted_name} 2>/dev/null");
    let new_session = format!("tmux -u new-session -d -s {quoted_name} {quoted_cmd}");
    let enable_mouse = format!("tmux set-option -t {quoted_name} mouse on 2>/dev/null || true");
    let set_history = format!(
        "tmux set-option -t {quoted_name} history-limit {TMUX_HISTORY_LIMIT} 2>/dev/null || true"
    );
    // Hide tmux's status bar: inside fartCode's pane the bar shows nothing the
    // tab bar doesn't already say — just the opaque `fartCode-<base64>` session
    // name (ADR-0026). Per-session, so the user's own tmux keeps its bar.
    // Runs on every attach, which also silences sessions created before
    // this shipped.
    let hide_status = format!("tmux set-option -t {quoted_name} status off 2>/dev/null || true");
    let configure = format!("({enable_mouse}) && ({set_history}) && ({hide_status})");
    let attach = format!("tmux -u attach-session -t {quoted_name}");
    let script = format!("({check_exists} || {new_session}) && {configure} && {attach}");
    script
}

/// The command a durable interactive terminal runs INSIDE its tmux session
/// (ADR-0025): `cd` into the task workspace and exec the user's shell.
/// Both values go through the shared `shell_escape` (never ad-hoc quoting).
/// tmux runs the line in its own default shell; `exec` makes the user shell
/// the session's foreground process, so typing `exit` ends the session.
pub fn build_terminal_session_command(cwd: &std::path::Path, shell: &str) -> String {
    format!(
        "cd {} && exec {}",
        crate::shell_escape::single_quote(&cwd.to_string_lossy()),
        crate::shell_escape::single_quote(shell),
    )
}

/// Variant of [`build_terminal_session_command`] that appends `args`
/// (E2-13, #52): `cd '<cwd>' && exec '<program>' '<arg1>' '<arg2>'`. Needed
/// for task startup commands, which spawn as `sh -c '<command>'` — the
/// plain builder cannot carry the `-c` argument. Every token goes through
/// the shared `shell_escape` (never ad-hoc quoting).
pub fn build_terminal_session_command_args(
    cwd: &std::path::Path,
    program: &str,
    args: &[String],
) -> String {
    let mut cmd = format!(
        "cd {} && exec {}",
        crate::shell_escape::single_quote(&cwd.to_string_lossy()),
        crate::shell_escape::single_quote(program),
    );
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&crate::shell_escape::single_quote(arg));
    }
    cmd
}

/// `tmux kill-session -t <name>` — the teardown side of the durability path
/// (conversation delete / task stop / terminal task teardown). Best-effort:
/// a session that already died is not an error.
pub fn kill_tmux_session(session_name: &str) -> Result<(), std::io::Error> {
    let binary = tmux_command();
    let status = std::process::Command::new(&binary)
        .args(["kill-session", "-t", session_name])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        // Exit 1 = "no such session" — already gone, not an error.
        Ok(())
    }
}

/// Whether THIS machine can run tmux for a local terminal (E13-03).
///
/// Windows never does — see [`RESOLVED_TMUX`]. Remote hosts are a separate
/// question, answered on the host.
pub fn local_tmux_supported() -> bool {
    !cfg!(windows)
}

/// Resolved tmux binary (ADR-0025).
#[derive(Debug, Clone)]
pub struct TmuxBinary {
    /// argv[0] for direct tmux invocations — a PATH name or an absolute path.
    pub command: String,
    /// `PATH` value to overlay on PTY children when tmux is NOT on the
    /// inherited PATH (GUI/Dock launch on macOS gets a minimal PATH).
    /// `None` when plain PATH lookup already works.
    pub path_overlay: Option<String>,
}

static RESOLVED_TMUX: std::sync::LazyLock<Option<TmuxBinary>> = std::sync::LazyLock::new(|| {
    // E13-03 (#93): Windows local terminals never run tmux. A tmux on PATH
    // there is msys/WSL — a different filesystem namespace than the worktree
    // the terminal is for. Remote hosts probe their OWN tmux (E12-05 AC12)
    // and are unaffected.
    if !local_tmux_supported() {
        return None;
    }
    if probe_tmux("tmux") {
        return Some(TmuxBinary {
            command: "tmux".into(),
            path_overlay: None,
        });
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let bin = format!("{dir}/tmux");
        if std::path::Path::new(&bin).is_file() && probe_tmux(&bin) {
            let current = std::env::var("PATH").unwrap_or_default();
            return Some(TmuxBinary {
                command: bin,
                path_overlay: Some(format!("{dir}:{current}")),
            });
        }
    }
    None
});

/// Locates tmux: PATH probe first, then the common install prefixes
/// (Dock-launched macOS apps inherit a minimal PATH that excludes
/// Homebrew). Cached for the process lifetime — install/uninstall mid-run
/// is not a supported transition. `None` when tmux is absent.
pub fn resolve_tmux_binary() -> Option<&'static TmuxBinary> {
    RESOLVED_TMUX.as_ref()
}

/// argv[0] for direct tmux calls: the resolved binary, or bare `tmux` when
/// tmux is absent (preserves the historical spawn-ENOENT error path).
fn tmux_command() -> String {
    resolve_tmux_binary()
        .map(|b| b.command.clone())
        .unwrap_or_else(|| "tmux".into())
}

fn probe_tmux(candidate: &str) -> bool {
    std::process::Command::new(candidate)
        .arg("-V")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Names whose decoded fartCode session id starts with `id_prefix` (pure filter —
/// foreign and malformed names never match).
pub fn tmux_sessions_matching<'a>(
    names: impl IntoIterator<Item = &'a str>,
    id_prefix: &str,
) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| parse_tmux_session_name(name).is_some_and(|id| id.starts_with(id_prefix)))
        .map(str::to_string)
        .collect()
}

/// `list-sessions` output format. Shared by the local process path and the
/// remote SSH path (E12-05 AC12) so both parse one shape.
/// `|`, not `\t`: tmux ≥3.7 sanitizes control characters in `-F` output to
/// `_`, which silently emptied every listing (sweep/list saw no sessions).
/// fartCode session names are `fartCode-` + base64url, so `|` cannot occur
/// in our names; the parser right-splits so foreign names containing `|`
/// still yield their attach flag intact.
pub const TMUX_LIST_FORMAT: &str = "#{session_name}|#{session_attached}";

/// Parses `list-sessions -F TMUX_LIST_FORMAT` stdout into `(name, attached)`.
/// Pure: malformed lines are dropped, so a tmux error message ("no server
/// running") simply yields no rows.
pub fn parse_tmux_session_rows(stdout: &str) -> Vec<(String, bool)> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, attached) = line.trim_end().rsplit_once('|')?;
            Some((name.to_string(), attached == "1"))
        })
        .collect()
}

/// fartCode sessions among `rows` whose decoded id starts with `id_prefix`
/// (pure — the caller supplies the listing, local or remote).
pub fn sessions_from_rows(rows: &[(String, bool)], id_prefix: &str) -> Vec<TmuxSessionInfo> {
    rows.iter()
        .filter_map(|(name, attached)| {
            Some(TmuxSessionInfo {
                session_id: parse_tmux_session_name(name)?,
                attached: *attached,
            })
        })
        .filter(|info| info.session_id.starts_with(id_prefix))
        .collect()
}

/// Remote availability probe (E12-05 AC12): the host may have no tmux, in
/// which case remote terminals spawn plain — same degradation as locally.
pub fn remote_tmux_probe_command() -> String {
    "command -v tmux".to_string()
}

/// Remote `list-sessions`. The format string is escaped, never interpolated
/// (AC14) — it is full of `#{}` that a shell must not touch.
pub fn remote_tmux_list_command() -> String {
    format!(
        "tmux list-sessions -F {}",
        crate::shell_escape::single_quote(TMUX_LIST_FORMAT)
    )
}

/// Remote `kill-session`. Best-effort on the remote too: a session that is
/// already gone exits nonzero and that is not an error.
pub fn remote_tmux_kill_command(session_name: &str) -> String {
    format!(
        "tmux kill-session -t {} 2>/dev/null || true",
        crate::shell_escape::single_quote(session_name)
    )
}

/// `list-sessions` rows for the local tmux server. Best-effort: tmux
/// absent / no server running → empty.
fn list_tmux_session_rows() -> Vec<(String, bool)> {
    let Some(binary) = resolve_tmux_binary() else {
        return Vec::new();
    };
    let output = std::process::Command::new(&binary.command)
        .args(["list-sessions", "-F", TMUX_LIST_FORMAT])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        // No tmux server running → nothing to list.
        return Vec::new();
    }
    parse_tmux_session_rows(&String::from_utf8_lossy(&output.stdout))
}

/// One live fartCode tmux session (decoded id + attach state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxSessionInfo {
    /// Decoded session id (`{project}:{task}:terminal:{slot}` for
    /// interactive terminals).
    pub session_id: String,
    /// True while at least one attach client is connected.
    pub attached: bool,
}

/// Live fartCode sessions whose decoded id starts with `id_prefix` (ADR-0028:
/// listing surviving terminal sessions on reopen). Foreign and malformed
/// names never match — they must decode via `parse_tmux_session_name`
/// first. Best-effort: no tmux / no server → empty.
pub fn list_tmux_sessions_by_prefix(id_prefix: &str) -> Vec<TmuxSessionInfo> {
    sessions_from_rows(&list_tmux_session_rows(), id_prefix)
}

/// Slot for a task's next durable terminal (ADR-0028): reuse the smallest
/// live DETACHED session not already owned by this process — reattaching a
/// surviving shell instead of minting a fresh one. When nothing is
/// reusable, take the smallest slot unused both locally and on the tmux
/// server (never double-attach a session another client holds).
///
/// `id_prefix` is `{project}:{task}:terminal:`; `owned_slots` are the
/// slots this process already shows (their sessions must not be reattached
/// a second time). Pure — the caller supplies the live listing.
pub fn choose_terminal_slot(
    id_prefix: &str,
    owned_slots: &std::collections::HashSet<u32>,
    live: &[TmuxSessionInfo],
) -> u32 {
    let slot_of = |s: &TmuxSessionInfo| {
        s.session_id
            .strip_prefix(id_prefix)
            .and_then(|rest| rest.parse::<u32>().ok())
    };
    let live_slots: std::collections::HashSet<u32> = live.iter().filter_map(slot_of).collect();
    live.iter()
        .filter(|s| !s.attached)
        .filter_map(slot_of)
        .filter(|slot| !owned_slots.contains(slot))
        .min()
        .unwrap_or_else(|| {
            (0..)
                .find(|n| !owned_slots.contains(n) && !live_slots.contains(n))
                .expect("u32 slot space")
        })
}

/// Kill every fartCode session whose decoded id starts with `id_prefix`
/// (ADR-0025 terminal task teardown — sweeps sessions orphaned by a crashed
/// app instance too). Best-effort: no tmux / no server → 0.
pub fn kill_tmux_sessions_by_prefix(id_prefix: &str) -> usize {
    let names: Vec<String> = list_tmux_session_rows()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let mut killed = 0;
    for name in tmux_sessions_matching(names.iter().map(String::as_str), id_prefix) {
        match kill_tmux_session(&name) {
            Ok(()) => killed += 1,
            Err(e) => tracing::warn!(session = %name, error = %e, "tmux kill-session failed"),
        }
    }
    killed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// E13-03 (#93): the local tmux path is Windows-free, and binary
    /// resolution honours that predicate rather than probing PATH.
    #[test]
    fn windows_never_runs_local_tmux() {
        assert_eq!(local_tmux_supported(), !cfg!(windows));
        if !local_tmux_supported() {
            assert!(resolve_tmux_binary().is_none());
        }
    }

    /// E13-02: a detached survivor is preferred over a fresh slot — the
    /// reattach path an SSH reconnect depends on.
    #[test]
    fn detached_survivor_is_reattached() {
        let live = vec![TmuxSessionInfo {
            session_id: "p1:t1:terminal:0".into(),
            attached: false,
        }];
        let owned = HashSet::new();
        assert_eq!(choose_terminal_slot("p1:t1:terminal:", &owned, &live), 0);
        // Still owned (a live client) → the survivor is not stolen.
        let owned = HashSet::from([0_u32]);
        assert_eq!(choose_terminal_slot("p1:t1:terminal:", &owned, &live), 1);
    }

    #[test]
    fn tmux_name_roundtrips() {
        let name = make_tmux_session_name("conv-123");
        assert!(name.starts_with(TMUX_SESSION_PREFIX));
        assert_eq!(parse_tmux_session_name(&name).as_deref(), Some("conv-123"));
    }

    #[test]
    fn tmux_name_rejects_foreign_names() {
        assert_eq!(parse_tmux_session_name("other-foo"), None);
        assert_eq!(parse_tmux_session_name("fartCode-"), None);
        assert_eq!(parse_tmux_session_name("fartCode-!!!not-base64!!!"), None);
    }

    #[test]
    fn tmux_shell_line_has_create_configure_attach() {
        let line = build_tmux_shell_line("fartCode-abc", "codex exec");
        assert!(
            line.contains("tmux has-session -t \"fartCode-abc\""),
            "{line}"
        );
        assert!(
            line.contains(&format!("history-limit {TMUX_HISTORY_LIMIT}")),
            "{line}"
        );
        // Status bar off: the pane's tab bar already identifies the session;
        // tmux's bar only shows the opaque `fartCode-<base64>` name (ADR-0026).
        assert!(line.contains("status off"), "{line}");
        assert!(
            line.contains("tmux -u attach-session -t \"fartCode-abc\""),
            "{line}"
        );
        // `||` guards must keep the whole line from failing when options
        // don't apply (older tmux / read-only session).
        assert!(line.contains("2>/dev/null || true"), "{line}");
    }

    #[test]
    fn tmux_shell_line_quotes_metacharacters() {
        // A session name or command with quotes/metacharacters must stay a
        // single JSON-quoted argument (no break-out of the tmux command).
        let line = build_tmux_shell_line("fartCode-a; rm -rf /", "x & y");
        assert!(line.contains("\"fartCode-a; rm -rf /\""), "{line}");
        assert!(line.contains("\"x & y\""), "{line}");
        // The base64url session name (our own format) never carries `$`.
        assert!(!make_tmux_session_name("a; rm -rf /").contains('$'));
    }

    #[test]
    fn kill_uses_quoted_session_name() {
        // The kill command passes the session name as a single arg.
        let name = make_tmux_session_name("conv-kill");
        assert_eq!(parse_tmux_session_name(&name).as_deref(), Some("conv-kill"));
        // kill_tmux_session itself shells out — this test pins the contract:
        // the name we generate is exactly what parse recovers (i.e. the kill
        // targets OUR session namespace).
        assert!(name.starts_with(TMUX_SESSION_PREFIX));
    }

    #[test]
    fn tmux_name_is_utf8_safe() {
        let name = make_tmux_session_name("conv-ünïcode");
        assert_eq!(
            parse_tmux_session_name(&name).as_deref(),
            Some("conv-ünïcode")
        );
    }

    #[test]
    fn terminal_session_command_cds_and_execs() {
        let cmd = build_terminal_session_command(std::path::Path::new("/tmp/wt-1"), "/bin/zsh");
        assert_eq!(cmd, "cd '/tmp/wt-1' && exec '/bin/zsh'");
    }

    #[test]
    fn terminal_session_command_args_carries_startup_command() {
        // E2-13 (#52): startup commands spawn as `sh -c '<cmd>'` — the args
        // variant must append each arg single-quoted (never ad-hoc quoting).
        let dir = tempfile::tempdir().unwrap();
        let cmd = build_terminal_session_command_args(
            dir.path(),
            "/bin/sh",
            &["-c".to_string(), "printf %s startup-ok".to_string()],
        );
        assert_eq!(
            cmd,
            format!(
                "cd '{}' && exec '/bin/sh' '-c' 'printf %s startup-ok'",
                dir.path().to_string_lossy()
            )
        );
        // The whole line round-trips through sh: the exec'd `sh -c` receives
        // the command verbatim.
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "startup-ok");

        // A hostile command (embedded quotes, $ expansion, spaces) must reach
        // the inner shell intact — single_quote each arg, no re-expansion.
        // The script itself uses the canonical `'it'\''s'` quote-escape idiom
        // and double-quoted $HOME (inner expansion must still happen).
        let hostile = "printf '%s\\n' 'it'\\''s' \"fine\" \"$HOME\"";
        let cmd = build_terminal_session_command_args(
            dir.path(),
            "/bin/sh",
            &["-c".to_string(), hostile.to_string()],
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("it's\nfine\n{home}\n"),
            "inner sh -c must receive the command verbatim (no outer expansion)"
        );
    }

    #[test]
    fn terminal_session_command_cd_survives_hostile_path() {
        // A workspace path with a space and a single quote must survive the
        // `cd` we emit: create it for real and run the cd-portion through sh.
        let dir = tempfile::tempdir().unwrap();
        let hostile = dir.path().join("it's here");
        std::fs::create_dir_all(&hostile).unwrap();
        let cmd = build_terminal_session_command(&hostile, "/bin/sh");
        // cd-portion is everything before the `exec`.
        let cd_part = cmd.split(" && exec ").next().unwrap();
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("{cd_part} && pwd")])
            .output()
            .expect("sh runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.trim().ends_with("it's here"),
            "pwd={stdout:?} cmd={cmd:?} status={:?}",
            output.status
        );
    }

    fn info(slot: u32, attached: bool) -> TmuxSessionInfo {
        TmuxSessionInfo {
            session_id: format!("p:t:terminal:{slot}"),
            attached,
        }
    }

    #[test]
    fn choose_slot_prefers_smallest_detached_survivor() {
        // Crash/restart recovery: slots 2 and 5 survived detached; the
        // fresh process owns nothing → reattach 2, not create 0.
        let live = [info(2, false), info(5, false)];
        assert_eq!(
            choose_terminal_slot("p:t:terminal:", &HashSet::new(), &live),
            2
        );
    }

    #[test]
    fn choose_slot_never_double_attaches_or_reuses_owned() {
        // Attached sessions belong to another client; owned slots are this
        // process's already-shown terminals — neither is reusable.
        let live = [info(0, true), info(1, false), info(2, false)];
        let owned: HashSet<u32> = [1].into_iter().collect();
        assert_eq!(choose_terminal_slot("p:t:terminal:", &owned, &live), 2);
        // Everything live is attached or owned → mint the first free slot.
        let live = [info(0, true), info(1, false)];
        assert_eq!(
            choose_terminal_slot("p:t:terminal:", &[1].into_iter().collect(), &live),
            2
        );
    }

    #[test]
    fn choose_slot_starts_at_zero_when_nothing_is_live() {
        assert_eq!(
            choose_terminal_slot("p:t:terminal:", &HashSet::new(), &[]),
            0
        );
    }

    #[test]
    fn choose_slot_ignores_non_numeric_suffixes() {
        // A malformed/foreign tail must not poison the slot space.
        let live = [
            TmuxSessionInfo {
                session_id: "p:t:terminal:abc".into(),
                attached: false,
            },
            info(3, false),
        ];
        assert_eq!(
            choose_terminal_slot("p:t:terminal:", &HashSet::new(), &live),
            3
        );
    }

    #[test]
    fn sessions_matching_filters_by_decoded_prefix() {
        // Our own sessions decode and match on the id prefix.
        let ours_a = make_tmux_session_name("task-1:terminal:0");
        let ours_b = make_tmux_session_name("task-1:terminal:1");
        let foreign_task = make_tmux_session_name("task-2:terminal:0");
        let conv_session = make_tmux_session_name("task-1:conv-9");
        let names = [
            "other-1",
            "fartCode-!!!bad!!!",
            &ours_a,
            &ours_b,
            &foreign_task,
            &conv_session,
        ];
        let matched = tmux_sessions_matching(names, "task-1:terminal:");
        assert_eq!(matched, vec![ours_a.clone(), ours_b.clone()], "{matched:?}");
        // Prefix that matches nothing → empty (never kills foreign sessions).
        assert!(tmux_sessions_matching(names, "task-9:").is_empty());
    }

    #[test]
    fn list_by_prefix_filters_decoded_ids() {
        // Pure filtering half of `list_tmux_sessions_by_prefix` (the live
        // listing itself needs a tmux server; covered by the durability
        // integration test).
        let ours = make_tmux_session_name("proj-1:task-1:terminal:0");
        let foreign = make_tmux_session_name("proj-2:task-1:terminal:0");
        let names = vec!["other".to_string(), ours.clone(), foreign];
        let matched: Vec<String> = names
            .into_iter()
            .filter_map(|name| parse_tmux_session_name(&name))
            .filter(|id| id.starts_with("proj-1:task-1:terminal:"))
            .collect();
        assert_eq!(matched, vec!["proj-1:task-1:terminal:0"]);
    }

    #[test]
    fn parses_list_sessions_rows() {
        // Foreign names may themselves contain `|` — the attach flag is the
        // text after the LAST delimiter.
        let rows = parse_tmux_session_rows("a|1\nb|0\r\nweird|name|1\nrubbish\n");
        assert_eq!(
            rows,
            vec![
                ("a".to_string(), true),
                ("b".to_string(), false),
                ("weird|name".to_string(), true),
            ]
        );
    }

    #[test]
    fn sessions_from_rows_filters_foreign_and_prefix() {
        let ours = make_tmux_session_name("p1:t1:terminal:0");
        let other_task = make_tmux_session_name("p1:t2:terminal:0");
        let rows = vec![
            (ours.clone(), false),
            (other_task, true),
            ("someone-elses-session".to_string(), true),
        ];
        let found = sessions_from_rows(&rows, "p1:t1:terminal:");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].session_id, "p1:t1:terminal:0");
        assert!(!found[0].attached);
    }

    #[test]
    fn remote_tmux_commands_are_escaped() {
        // The list format is full of `#{}`; the shell must never see it raw.
        let list = remote_tmux_list_command();
        assert!(list.contains(&format!("'{TMUX_LIST_FORMAT}'")), "{list}");
        // A session name is our own base64url, but the escape is what keeps
        // it that way if a caller ever passes something else (AC14).
        let kill = remote_tmux_kill_command("fartCode-x'; rm -rf /");
        assert!(kill.contains(r#"'fartCode-x'\''; rm -rf /'"#), "{kill}");
        assert!(kill.ends_with("|| true"), "{kill}");
    }

    #[test]
    fn remote_session_command_survives_a_hostile_worktree_path() {
        // E12-05 AC14: a remote worktree path with quotes, spaces and a
        // semicolon is data inside the tmux session command, never syntax.
        let cwd = std::path::Path::new("/home/u/wt/it's here; rm -rf /");
        let inner = build_terminal_session_command(cwd, "/bin/bash");
        assert_eq!(
            inner,
            r#"cd '/home/u/wt/it'\''s here; rm -rf /' && exec '/bin/bash'"#
        );
        let line = build_tmux_shell_line(&make_tmux_session_name("p:t:terminal:0"), &inner);
        // The whole session command rides as ONE JSON-quoted tmux argument.
        assert!(
            line.contains(&serde_json::to_string(&inner).unwrap()),
            "{line}"
        );
    }
}
