//! Tmux durability (E2-07): session naming + the create-or-attach shell line.
//! Port of `tmux-session-name.ts`. The tmux ENABLED path is what survives a
//! hard kill of the ade process (the tmux server owns the session); the
//! non-tmux fallback rehydrates best-effort on boot.

/// `TMUX_SESSION_PREFIX` (reference `emdash-`, ours `ade-`).
pub const TMUX_SESSION_PREFIX: &str = "ade-";
/// Reference `TMUX_HISTORY_LIMIT`.
pub const TMUX_HISTORY_LIMIT: u32 = 100_000;

/// `makeTmuxSessionName`: `ade-` + base64url(sessionId).
pub fn make_tmux_session_name(session_id: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    format!(
        "{TMUX_SESSION_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(session_id.as_bytes())
    )
}

/// Inverse of `make_tmux_session_name`: returns the decoded session id, or
/// `None` when the name isn't a well-formed ade tmux session name.
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
pub fn build_tmux_shell_line(session_name: &str, command_line: &str) -> String {
    let check_exists = format!("tmux has-session -t {session_name} 2>/dev/null");
    let new_session = format!("tmux -u new-session -d -s {session_name} {command_line}");
    let enable_mouse = format!("tmux set-option -t {session_name} mouse on 2>/dev/null || true");
    let set_history = format!(
        "tmux set-option -t {session_name} history-limit {TMUX_HISTORY_LIMIT} 2>/dev/null || true"
    );
    let configure = format!("({enable_mouse}) && ({set_history})");
    let attach = format!("tmux -u attach-session -t {session_name}");
    let script = format!("({check_exists} || {new_session}) && {configure} && {attach}");
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_name_roundtrips() {
        let name = make_tmux_session_name("conv-123");
        assert!(name.starts_with(TMUX_SESSION_PREFIX));
        assert_eq!(parse_tmux_session_name(&name).as_deref(), Some("conv-123"));
    }

    #[test]
    fn tmux_name_rejects_foreign_names() {
        assert_eq!(parse_tmux_session_name("other-foo"), None);
        assert_eq!(parse_tmux_session_name("ade-"), None);
        assert_eq!(parse_tmux_session_name("ade-!!!not-base64!!!"), None);
    }

    #[test]
    fn tmux_shell_line_has_create_configure_attach() {
        let line = build_tmux_shell_line("ade-abc", "codex exec");
        assert!(line.contains("tmux has-session -t ade-abc"), "{line}");
        assert!(
            line.contains("tmux -u new-session -d -s ade-abc codex exec"),
            "{line}"
        );
        assert!(line.contains("mouse on"), "{line}");
        assert!(
            line.contains(&format!("history-limit {TMUX_HISTORY_LIMIT}")),
            "{line}"
        );
        assert!(line.contains("tmux -u attach-session -t ade-abc"), "{line}");
        // `||` guards must keep the whole line from failing when options
        // don't apply (older tmux / read-only session).
        assert!(line.contains("2>/dev/null || true"), "{line}");
    }

    #[test]
    fn tmux_name_is_utf8_safe() {
        let name = make_tmux_session_name("conv-ünïcode");
        assert_eq!(
            parse_tmux_session_name(&name).as_deref(),
            Some("conv-ünïcode")
        );
    }
}
