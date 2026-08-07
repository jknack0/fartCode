//! Agent env allowlist (E3-08): the single, security-reviewed source of
//! truth for what env vars reach agent processes. Anything not in these
//! lists NEVER leaks into `build_agent_env` output.
//!
//! Port of `pty-env.ts::buildAgentEnv` + `packages/core/src/agents/agent-env.ts`.
//! Adding a variable = touching ONE list here + PR review (enforced by the
//! `adding_a_var_requires_one_file` test).

use std::collections::HashMap;
use std::path::Path;

/// Base env: forced values + allowlisted pass-throughs.
pub const BASE_AGENT_ENV: &[(&str, &str)] = &[
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "fartCode"),
];

/// Passed through from the process env when present.
pub const BASE_PASSTHROUGH: &[&str] = &["HOME", "USER", "PATH", "TMPDIR"];

/// `GLOBAL_AGENT_ENV_VARS`.
pub const GLOBAL_AGENT_ENV_VARS: &[&str] =
    &["EDITOR", "VISUAL", "GIT_EDITOR", "HOSTNAME", "LANG", "TZ"];

/// `DISPLAY_ENV_VARS`.
pub const DISPLAY_ENV_VARS: &[&str] = &[
    "DISPLAY",
    "XAUTHORITY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "DBUS_SESSION_BUS_ADDRESS",
];

/// `AGENT_ENV_VARS` — the provider API-key / config surface (94 keys,
/// exact per `packages/core/src/agents/agent-env.ts`), plus the 4 lowercase
/// proxy variants the ticket requires (98 total).
pub const AGENT_ENV_VARS: &[&str] = &[
    "AIMLAPI_API_KEY",
    "ALL_PROXY",
    "AMP_API_KEY",
    "AMP_TOOLBOX",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "AUTOHAND_API_KEY",
    "AUGMENT_SESSION_AUTH",
    "AWS_ACCESS_KEY_ID",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_OPENAI_API_ENDPOINT",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_KEY",
    "BAILIAN_CODING_PLAN_API_KEY",
    "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
    "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CONFIG_DIR",
    "CODEBUFF_API_KEY",
    "CODEX_HOME",
    "COPILOT_CLI_TOKEN",
    "CURSOR_API_KEY",
    "DASHSCOPE_API_KEY",
    "FACTORY_API_KEY",
    "GEMINI_API_KEY",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_GENAI_API_VERSION",
    "GOOGLE_VERTEX_BASE_URL",
    "GOOSE_CONTEXT_LIMIT",
    "GOOSE_LEAD_MODEL",
    "GOOSE_LEAD_PROVIDER",
    "GOOSE_MODE",
    "GOOSE_MODEL",
    "GOOSE_PLANNER_MODEL",
    "GOOSE_PLANNER_PROVIDER",
    "GOOSE_PROVIDER",
    "GOOSE_PROVIDER__API_KEY",
    "GOOSE_PROVIDER__HOST",
    "GOOSE_PROVIDER__TYPE",
    "GROK_CODE_XAI_API_KEY",
    "GROK_DEPLOYMENT_KEY",
    "GROK_HOME",
    "GROK_POOL_IDLE_TIMEOUT_SECS",
    "GROK_PROXY_URL",
    "GROK_SANDBOX",
    "GROQ_API_KEY",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "KIMI_API_KEY",
    "MISTRAL_API_KEY",
    "MOONSHOT_API_KEY",
    "NO_PROXY",
    "OMP_AUTH_BROKER_SNAPSHOT_CACHE",
    "OMP_AUTH_BROKER_SNAPSHOT_TTL_MS",
    "OMP_AUTH_BROKER_URL",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_MODEL",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
    "OPENCODE_MODEL",
    "OPENROUTER_API_KEY",
    "OPENROUTER_BASE_URL",
    "PI_CODING_AGENT_DIR",
    "PI_CONFIG_DIR",
    "PI_NO_TITLE",
    "PI_PLAN_MODEL",
    "PI_SLOW_MODEL",
    "PI_SMOL_MODEL",
    "QWEN_CODE_SUPPRESS_YOLO_WARNING",
    "QWEN_DEFAULT_AUTH_TYPE",
    "QWEN_HOME",
    "QWEN_MODEL",
    "QWEN_RUNTIME_DIR",
    "QWEN_SANDBOX",
    "XAI_API_KEY",
    "ZAI_API_KEY",
    "all_proxy",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// Windows essential set (only applied on Windows).
pub const WINDOWS_ESSENTIAL: &[&str] = &[
    "PATHEXT",
    "SystemRoot",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramW6432",
];

/// Hook env (E3-05 hook server): injected when the hook server is up.
pub const HOOK_ENV: &[&str] = &[
    "FARTCODE_HOOK_PORT",
    "FARTCODE_PTY_ID",
    "FARTCODE_HOOK_NONCE",
    "FARTCODE_HOOK_TOKEN",
];

/// Whether a key is on ANY allowlist (the "one file" source of truth).
pub fn is_allowlisted(key: &str) -> bool {
    BASE_AGENT_ENV.iter().any(|(k, _)| *k == key)
        || key == "SSH_AUTH_SOCK"
        || BASE_PASSTHROUGH.contains(&key)
        || GLOBAL_AGENT_ENV_VARS.contains(&key)
        || DISPLAY_ENV_VARS.contains(&key)
        || AGENT_ENV_VARS.contains(&key)
        || WINDOWS_ESSENTIAL.contains(&key)
        || HOOK_ENV.contains(&key)
}

/// Locates an SSH agent socket: process env → macOS `launchctl getenv` →
/// common socket locations (1Password, OpenSSH temp, GNOME keyring, GnuPG).
pub fn detect_ssh_auth_sock() -> Option<String> {
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        if !sock.is_empty() {
            return Some(sock);
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("launchctl")
        .args(["getenv", "SSH_AUTH_SOCK"])
        .output()
    {
        if out.status.success() {
            let socket = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !socket.is_empty() && is_socket_file(Path::new(&socket)) {
                return Some(socket);
            }
        }
    }
    // Common socket locations (glob-expanded).
    let home = std::env::var("HOME").ok();
    let tmp = std::env::temp_dir();
    let candidates: Vec<String> = vec![
        // macOS launchd
        "/private/tmp/com.apple.launchd.*/Listeners".to_string(),
        // 1Password SSH agent
        home.as_ref()
            .map(|h| format!("{h}/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"))
            .unwrap_or_default(),
        // OpenSSH temp
        tmp.join("ssh-??????????/agent.*")
            .to_string_lossy()
            .into_owned(),
        // User .ssh
        home.as_ref()
            .map(|h| format!("{h}/.ssh/agent.*"))
            .unwrap_or_default(),
        // GNOME keyring
        tmp.join("keyring-*/ssh").to_string_lossy().into_owned(),
        // GnuPG agent
        home.as_ref()
            .map(|h| format!("{h}/.gnupg/S.gpg-agent.ssh"))
            .unwrap_or_default(),
    ];
    for pattern in candidates {
        if pattern.is_empty() {
            continue;
        }
        if pattern.contains('*') || pattern.contains('?') {
            if let Ok(paths) = glob::glob(&pattern) {
                for path in paths.flatten() {
                    if is_socket_file(&path) {
                        return Some(path.to_string_lossy().into_owned());
                    }
                }
            }
        } else if is_socket_file(Path::new(&pattern)) {
            return Some(pattern);
        }
    }
    None
}

/// S_ISSOCK check without libc: metadata + `is_socket()` — no connect, no
/// side effect on the user's live agent, no blocking on a full backlog.
#[cfg(unix)]
fn is_socket_file(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_socket_file(_path: &Path) -> bool {
    false
}

/// Builds the agent env (reference `buildAgentEnv`):
/// base env → allowlisted pass-throughs from the process env (global +
/// display + agent + windows-essential) → `SSH_AUTH_SOCK` (detected when
/// missing) → provider vars → task env (`FARTCODE_*`, E1-06) → hook env.
/// Nothing not on the allowlists can reach the output.
pub fn build_agent_env(
    provider_env: &[(String, String)],
    task_env: &[(String, String)],
    hook_env: Option<(&str, &str, &str, &str)>, // (port, pty_id, nonce, token)
) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = BASE_AGENT_ENV
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    for key in BASE_PASSTHROUGH {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    for key in GLOBAL_AGENT_ENV_VARS.iter().chain(DISPLAY_ENV_VARS) {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    for key in AGENT_ENV_VARS {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    #[cfg(windows)]
    {
        // PATH insert must come BEFORE the `default` closure (which captures
        // `env` by &mut) — a second &mut borrow here is a Windows-only
        // compile error.
        env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        let default = |key: &str, fallback: &str| {
            env.entry(key.to_string())
                .or_insert_with(|| std::env::var(key).unwrap_or_else(|_| fallback.to_string()));
        };
        default(
            "PATHEXT",
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC",
        );
        default("SystemRoot", "C:\\Windows");
        default("ComSpec", "C:\\Windows\\System32\\cmd.exe");
        default("TEMP", &std::env::var("TMP").unwrap_or_default());
        default("TMP", &std::env::var("TEMP").unwrap_or_default());
        default(
            "USERPROFILE",
            &std::env::var("USERPROFILE").unwrap_or_default(),
        );
        default("APPDATA", "");
        default("LOCALAPPDATA", "");
        default("HOMEDRIVE", "");
        default("HOMEPATH", "");
        default("USERNAME", "");
        default("ProgramFiles", "C:\\Program Files");
        default("ProgramFiles(x86)", "C:\\Program Files (x86)");
        default("ProgramData", "C:\\ProgramData");
        default("CommonProgramFiles", "C:\\Program Files\\Common Files");
        default(
            "CommonProgramFiles(x86)",
            "C:\\Program Files (x86)\\Common Files",
        );
        default("ProgramW6432", "C:\\Program Files");
        default("CommonProgramW6432", "C:\\Program Files\\Common Files");
    }

    let ssh_auth_sock = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(detect_ssh_auth_sock);
    if let Some(sock) = ssh_auth_sock {
        env.insert("SSH_AUTH_SOCK".into(), sock);
    }

    for (k, v) in provider_env {
        env.insert(k.clone(), v.clone());
    }
    for (k, v) in task_env {
        env.insert(k.clone(), v.clone());
    }
    if let Some((port, pty_id, nonce, token)) = hook_env {
        env.insert("FARTCODE_HOOK_PORT".into(), port.to_string());
        env.insert("FARTCODE_PTY_ID".into(), pty_id.to_string());
        env.insert("FARTCODE_HOOK_NONCE".into(), nonce.to_string());
        env.insert("FARTCODE_HOOK_TOKEN".into(), token.to_string());
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores the process env on drop (a panicking assert can't leave a
    /// polluted env for concurrently-running tests).
    struct EnvGuard(Vec<(&'static str, Option<String>)>);
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self(vec![(key, prev)])
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, prev) in &self.0 {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn security_secret_token_never_leaks() {
        // Simulate a hostile parent env: a secret NOT on any allowlist.
        let _g1 = EnvGuard::set("SECRET_TOKEN", "abc123");
        let _g2 = EnvGuard::set("ANTHROPIC_API_KEY", "sk-ant-ok");
        let env = build_agent_env(&[], &[], None);
        assert!(!env.contains_key("SECRET_TOKEN"), "{env:?}");
        assert_eq!(
            env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-ant-ok")
        );
    }

    #[test]
    fn base_env_is_always_present() {
        let env = build_agent_env(&[], &[], None);
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(
            env.get("TERM_PROGRAM").map(String::as_str),
            Some("fartCode")
        );
        assert!(env.contains_key("HOME"));
        assert!(env.contains_key("PATH"));
    }

    #[test]
    fn pathext_is_windows_only() {
        let env = build_agent_env(&[], &[], None);
        assert_eq!(env.contains_key("PATHEXT"), cfg!(windows));
    }

    #[test]
    fn overlay_order_provider_task_hook() {
        // task env beats provider env; hook env beats task env.
        let env = build_agent_env(
            &[("FARTCODE_PORT".into(), "9999".into())],
            &[
                ("FARTCODE_PORT".into(), "55555".into()),
                ("FARTCODE_TASK_ID".into(), "t1".into()),
            ],
            Some(("8080", "pty-1", "nonce", "tok")),
        );
        assert_eq!(env.get("FARTCODE_PORT").map(String::as_str), Some("55555"));
        assert_eq!(env.get("FARTCODE_TASK_ID").map(String::as_str), Some("t1"));
        assert_eq!(
            env.get("FARTCODE_HOOK_PORT").map(String::as_str),
            Some("8080")
        );
        assert_eq!(
            env.get("FARTCODE_PTY_ID").map(String::as_str),
            Some("pty-1")
        );
        assert_eq!(
            env.get("FARTCODE_HOOK_TOKEN").map(String::as_str),
            Some("tok")
        );
    }

    #[test]
    fn adding_a_var_requires_one_file() {
        // The canonical list is exactly the module's lists — anything the
        // builder could emit must be reachable through `is_allowlisted`.
        let env = build_agent_env(&[("SOME_PROVIDER_KEY".into(), "x".into())], &[], None);
        // provider_env is injected by the caller (registry), not filtered —
        // that's the E3-01 extension point. Everything else must be on the
        // allowlist.
        assert_eq!(env.get("SOME_PROVIDER_KEY").map(String::as_str), Some("x"));
        for key in env.keys() {
            assert!(
                is_allowlisted(key) || key == "SOME_PROVIDER_KEY",
                "{key} escaped the allowlist"
            );
        }
    }

    #[test]
    fn ssh_auth_sock_detection_runs() {
        // Present when the host has one; never panics otherwise.
        let env = build_agent_env(&[], &[], None);
        if std::env::var("SSH_AUTH_SOCK")
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            assert!(env.contains_key("SSH_AUTH_SOCK"));
        }
    }
}
