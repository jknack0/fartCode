//! OpenSSH config resolution (E12-03).
//!
//! `ssh -G <alias>` is the canonical source of truth at connect time: it applies
//! `Host`/`Match` blocks, includes, and defaults exactly the way the user's own
//! `ssh` binary would. We parse its output rather than reimplementing
//! `~/.ssh/config` semantics.
//!
//! Security: the alias lands in argv (never a shell string) and is validated
//! first, so it cannot masquerade as an `ssh` flag.

use std::process::Stdio;
use std::time::Duration;

use fartcode_core::Error;
use tokio::process::Command;
use tracing::debug;

use crate::{AuthMethod, ConnectionParams};

/// Wall-clock cap for one `ssh -G` invocation.
const SSH_G_TIMEOUT: Duration = Duration::from_secs(10);
/// Ceiling on `ssh -G` stdout; real output is ~2KB.
const MAX_OUTPUT: usize = 256 * 1024;

// ── Resolved config ──────────────────────────────────────

/// The subset of `ssh -G` output we act on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedSshConfig {
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity_files: Vec<String>,
    pub identity_agent: Option<String>,
    /// `IdentityAgent none` — agent auth explicitly turned off for this host.
    pub identity_agent_disabled: bool,
    pub identities_only: bool,
    pub proxy_jump: Option<String>,
    pub proxy_command: Option<String>,
    pub forward_agent: bool,
    pub connect_timeout: Option<u32>,
    pub server_alive_interval: Option<u32>,
    pub server_alive_count_max: Option<u32>,
}

impl ResolvedSshConfig {
    /// Build connection params for [`crate::SshClient::connect`].
    ///
    /// The resolved profile wins over anything the user typed: `ssh -G` already
    /// folded in their overrides, so re-applying manual fields here would
    /// silently diverge from what `ssh <alias>` does.
    pub fn to_params(&self, auth: AuthMethod) -> Result<ConnectionParams, Error> {
        if self.hostname.is_empty() {
            return Err(Error::SshConnection(
                "ssh -G returned no hostname".to_string(),
            ));
        }
        if self.user.is_empty() {
            return Err(Error::SshConnection(format!(
                "ssh -G returned no user for {}",
                self.hostname
            )));
        }
        Ok(ConnectionParams {
            host: self.hostname.clone(),
            port: self.port,
            username: self.user.clone(),
            auth,
            forward_agent: self.forward_agent,
        })
    }

    /// First identity file, if any — the usual pick for key auth.
    pub fn primary_identity_file(&self) -> Option<&str> {
        self.identity_files.first().map(String::as_str)
    }
}

// ── Alias validation ─────────────────────────────────────

fn alias_char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._@%+:/[]-".contains(c)
}

/// Validates a host alias before it becomes an `ssh` argument.
///
/// Rejects empty aliases, anything starting with `-` (would be read as a flag),
/// and characters outside OpenSSH's host syntax.
pub fn validate_alias(alias: &str) -> Result<&str, Error> {
    let trimmed = alias.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || !trimmed.chars().all(alias_char_ok) {
        return Err(Error::SshConnection(format!(
            "invalid SSH config alias: {alias:?}"
        )));
    }
    Ok(trimmed)
}

// ── Parsing ─────────────────────────────────────────────

/// `none` is OpenSSH's "explicitly unset" sentinel for path-ish directives.
fn optional_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse `ssh -G` stdout. Keys are case-insensitive; for repeated keys the last
/// occurrence wins, except `identityfile`, which accumulates.
pub fn parse_ssh_g(output: &str) -> ResolvedSshConfig {
    let mut cfg = ResolvedSshConfig {
        port: 22,
        ..Default::default()
    };

    for line in output.lines() {
        let line = line.trim();
        let Some(split) = line.find(char::is_whitespace) else {
            continue;
        };
        let key = line[..split].to_ascii_lowercase();
        let value = line[split..].trim();
        if value.is_empty() {
            continue;
        }

        match key.as_str() {
            "hostname" => cfg.hostname = optional_value(value).unwrap_or_default(),
            "user" => cfg.user = optional_value(value).unwrap_or_default(),
            "port" => cfg.port = value.parse().unwrap_or(22),
            "identityfile" => cfg.identity_files.push(value.to_string()),
            "identityagent" => {
                cfg.identity_agent_disabled = value.eq_ignore_ascii_case("none");
                cfg.identity_agent = optional_value(value);
            }
            "identitiesonly" => cfg.identities_only = value.eq_ignore_ascii_case("yes"),
            "proxyjump" => cfg.proxy_jump = optional_value(value),
            "proxycommand" => cfg.proxy_command = optional_value(value),
            // Anything but `no` enables forwarding (OpenSSH also accepts a
            // socket path here, which implies yes).
            "forwardagent" => cfg.forward_agent = !value.eq_ignore_ascii_case("no"),
            "connecttimeout" => cfg.connect_timeout = value.parse().ok(),
            "serveraliveinterval" => cfg.server_alive_interval = value.parse().ok(),
            "serveralivecountmax" => cfg.server_alive_count_max = value.parse().ok(),
            _ => {}
        }
    }

    cfg
}

/// Run `ssh -G <alias>` and parse the result.
pub async fn resolve_ssh_config(alias: &str) -> Result<ResolvedSshConfig, Error> {
    resolve_ssh_config_with(alias, "ssh").await
}

/// [`resolve_ssh_config`] with an explicit `ssh` binary path.
pub async fn resolve_ssh_config_with(
    alias: &str,
    ssh_path: &str,
) -> Result<ResolvedSshConfig, Error> {
    let alias = validate_alias(alias)?;

    let child = Command::new(ssh_path)
        .arg("-G")
        .arg(alias)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // On timeout the future is dropped, which drops the child, which kills
        // it — no orphaned `ssh` processes.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::SshConnection(format!("spawn {ssh_path} -G {alias}: {e}")))?;

    let output = tokio::time::timeout(SSH_G_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            Error::SshConnection(format!(
                "ssh -G {alias} timed out after {}s",
                SSH_G_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| Error::SshConnection(format!("ssh -G {alias}: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("exit {}", output.status)
        } else {
            stderr
        };
        return Err(Error::SshConnection(format!("ssh -G {alias}: {detail}")));
    }

    if output.stdout.len() > MAX_OUTPUT {
        return Err(Error::SshConnection(format!(
            "ssh -G {alias}: output exceeds {MAX_OUTPUT} bytes"
        )));
    }

    let cfg = parse_ssh_g(&String::from_utf8_lossy(&output.stdout));
    debug!(alias, host = %cfg.hostname, port = cfg.port, "resolved ssh config");
    Ok(cfg)
}

// ── Agent socket ────────────────────────────────────────

/// Outcome of resolving which agent socket to use for a host.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentSocket {
    /// Use this socket path.
    Socket(String),
    /// `IdentityAgent none` — do not use an agent.
    Disabled,
    /// No `IdentityAgent` and no `SSH_AUTH_SOCK`.
    Unset,
    /// Config and environment disagree. Never guess: agent forwarding with the
    /// wrong socket leaks the wrong keys to the remote host.
    Ambiguous { config: String, process: String },
}

/// Expand `SSH_AUTH_SOCK`, `$VAR`, `${VAR}`; anything else is a literal path.
fn expand_agent_path(value: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let name = if value == "SSH_AUTH_SOCK" {
        Some("SSH_AUTH_SOCK")
    } else if let Some(rest) = value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        Some(rest)
    } else {
        value.strip_prefix('$')
    };

    match name {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            env(n)
        }
        _ => Some(value.to_string()),
    }
}

/// Resolve the agent socket for a host, flagging config/env disagreement.
pub fn agent_socket(cfg: &ResolvedSshConfig, env: &dyn Fn(&str) -> Option<String>) -> AgentSocket {
    if cfg.identity_agent_disabled {
        return AgentSocket::Disabled;
    }

    let process_sock = env("SSH_AUTH_SOCK").filter(|s| !s.is_empty());

    let Some(configured) = cfg.identity_agent.as_deref() else {
        return match process_sock {
            Some(sock) => AgentSocket::Socket(sock),
            None => AgentSocket::Unset,
        };
    };

    match expand_agent_path(configured, env).filter(|s| !s.is_empty()) {
        None => match process_sock {
            Some(sock) => AgentSocket::Socket(sock),
            None => AgentSocket::Unset,
        },
        Some(sock) => match process_sock {
            Some(proc_sock) if proc_sock != sock => AgentSocket::Ambiguous {
                config: sock,
                process: proc_sock,
            },
            _ => AgentSocket::Socket(sock),
        },
    }
}

/// [`agent_socket`] against the current process environment.
pub fn agent_socket_from_env(cfg: &ResolvedSshConfig) -> AgentSocket {
    agent_socket(cfg, &|name| std::env::var(name).ok())
}

// ── ProxyJump ──────────────────────────────────────────

/// One hop of a `ProxyJump` chain.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpHop {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
}

/// Parse `ProxyJump` (`[user@]host[:port]`, comma-separated, IPv6 in brackets).
pub fn parse_proxy_jump(spec: &str) -> Result<Vec<JumpHop>, Error> {
    let mut hops = Vec::new();

    for raw in spec.split(',') {
        let hop = raw.trim();
        if hop.is_empty() {
            return Err(Error::SshConnection(format!(
                "invalid ProxyJump spec: {spec:?}"
            )));
        }

        let (user, rest) = match hop.rsplit_once('@') {
            Some((u, r)) if !u.is_empty() => (Some(u.to_string()), r),
            _ => (None, hop),
        };

        let (host, port) = if let Some(rest) = rest.strip_prefix('[') {
            // Bracketed IPv6: [::1] or [::1]:2222
            let (host, tail) = rest.split_once(']').ok_or_else(|| {
                Error::SshConnection(format!("unterminated IPv6 host in ProxyJump: {hop:?}"))
            })?;
            let port = match tail.strip_prefix(':') {
                Some(p) => parse_port(p, hop)?,
                None if tail.is_empty() => 22,
                None => {
                    return Err(Error::SshConnection(format!(
                        "invalid ProxyJump hop: {hop:?}"
                    )))
                }
            };
            (host.to_string(), port)
        } else {
            match rest.split_once(':') {
                Some((h, p)) => (h.to_string(), parse_port(p, hop)?),
                None => (rest.to_string(), 22),
            }
        };

        if host.is_empty() {
            return Err(Error::SshConnection(format!(
                "invalid ProxyJump hop: {hop:?}"
            )));
        }

        hops.push(JumpHop { user, host, port });
    }

    Ok(hops)
}

fn parse_port(value: &str, hop: &str) -> Result<u16, Error> {
    value
        .parse()
        .map_err(|_| Error::SshConnection(format!("invalid port in ProxyJump hop: {hop:?}")))
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
host example\n\
hostname example.internal\n\
user deploy\n\
port 2222\n\
identityfile ~/.ssh/id_ed25519\n\
identityfile ~/.ssh/id_rsa\n\
identityagent none\n\
identitiesonly yes\n\
proxyjump bastion\n\
forwardagent yes\n\
connecttimeout 7\n\
serveraliveinterval 15\n\
serveralivecountmax 3\n";

    fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn parses_ssh_g_output() {
        let cfg = parse_ssh_g(SAMPLE);
        assert_eq!(cfg.hostname, "example.internal");
        assert_eq!(cfg.user, "deploy");
        assert_eq!(cfg.port, 2222);
        assert_eq!(cfg.identity_files.len(), 2);
        assert_eq!(cfg.primary_identity_file(), Some("~/.ssh/id_ed25519"));
        assert!(cfg.identity_agent_disabled);
        assert_eq!(cfg.identity_agent, None);
        assert!(cfg.identities_only);
        assert_eq!(cfg.proxy_jump.as_deref(), Some("bastion"));
        assert!(cfg.forward_agent);
        assert_eq!(cfg.connect_timeout, Some(7));
        assert_eq!(cfg.server_alive_interval, Some(15));
        assert_eq!(cfg.server_alive_count_max, Some(3));
    }

    #[test]
    fn last_value_wins_and_defaults_apply() {
        let cfg = parse_ssh_g("user first\nUser second\nproxycommand none\ngarbage\n");
        assert_eq!(cfg.user, "second");
        assert_eq!(cfg.port, 22, "port defaults to 22 when absent");
        assert_eq!(cfg.proxy_command, None, "`none` means unset");
        assert!(!cfg.forward_agent);
    }

    #[test]
    fn forward_agent_socket_value_counts_as_enabled() {
        assert!(parse_ssh_g("forwardagent /tmp/agent.sock\n").forward_agent);
        assert!(!parse_ssh_g("forwardagent no\n").forward_agent);
    }

    #[test]
    fn alias_validation_rejects_flags_and_junk() {
        assert_eq!(validate_alias("  prod-box ").unwrap(), "prod-box");
        assert!(validate_alias("user@host:22").is_ok());
        for bad in [
            "",
            "   ",
            "-oProxyCommand=touch /tmp/pwned",
            "a b",
            "a;b",
            "$(id)",
        ] {
            assert!(validate_alias(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn to_params_requires_hostname_and_user() {
        let cfg = parse_ssh_g(SAMPLE);
        let params = cfg.to_params(AuthMethod::Agent).unwrap();
        assert_eq!(params.host, "example.internal");
        assert_eq!(params.port, 2222);
        assert_eq!(params.username, "deploy");

        assert!(parse_ssh_g("user deploy\n")
            .to_params(AuthMethod::Agent)
            .is_err());
        assert!(parse_ssh_g("hostname h\n")
            .to_params(AuthMethod::Agent)
            .is_err());
    }

    #[test]
    fn agent_socket_disabled_and_unset() {
        let disabled = parse_ssh_g("identityagent none\n");
        assert_eq!(
            agent_socket(&disabled, &env_from(&[("SSH_AUTH_SOCK", "/tmp/a")])),
            AgentSocket::Disabled
        );

        let plain = parse_ssh_g("hostname h\n");
        assert_eq!(agent_socket(&plain, &env_from(&[])), AgentSocket::Unset);
        assert_eq!(
            agent_socket(&plain, &env_from(&[("SSH_AUTH_SOCK", "/tmp/a")])),
            AgentSocket::Socket("/tmp/a".into())
        );
    }

    #[test]
    fn agent_socket_expands_variables() {
        let env = env_from(&[("SSH_AUTH_SOCK", "/tmp/a"), ("MY_SOCK", "/tmp/a")]);
        for spec in ["SSH_AUTH_SOCK", "$MY_SOCK", "${MY_SOCK}", "/tmp/a"] {
            let cfg = parse_ssh_g(&format!("identityagent {spec}\n"));
            assert_eq!(
                agent_socket(&cfg, &env),
                AgentSocket::Socket("/tmp/a".into()),
                "spec {spec:?}"
            );
        }

        // Unset variable falls back to the process socket.
        let cfg = parse_ssh_g("identityagent $NOPE\n");
        assert_eq!(
            agent_socket(&cfg, &env),
            AgentSocket::Socket("/tmp/a".into())
        );
    }

    #[test]
    fn agent_socket_ambiguity_is_reported_not_guessed() {
        let cfg = parse_ssh_g("identityagent /tmp/config.sock\n");
        assert_eq!(
            agent_socket(&cfg, &env_from(&[("SSH_AUTH_SOCK", "/tmp/process.sock")])),
            AgentSocket::Ambiguous {
                config: "/tmp/config.sock".into(),
                process: "/tmp/process.sock".into(),
            }
        );

        // No process socket — nothing to disagree with.
        assert_eq!(
            agent_socket(&cfg, &env_from(&[])),
            AgentSocket::Socket("/tmp/config.sock".into())
        );
    }

    #[test]
    fn parses_proxy_jump_chain() {
        let hops = parse_proxy_jump("alice@bastion:2200, relay ,[fd00::1]:2222,[fd00::2]").unwrap();
        assert_eq!(
            hops,
            vec![
                JumpHop {
                    user: Some("alice".into()),
                    host: "bastion".into(),
                    port: 2200
                },
                JumpHop {
                    user: None,
                    host: "relay".into(),
                    port: 22
                },
                JumpHop {
                    user: None,
                    host: "fd00::1".into(),
                    port: 2222
                },
                JumpHop {
                    user: None,
                    host: "fd00::2".into(),
                    port: 22
                },
            ]
        );
    }

    #[test]
    fn rejects_bad_proxy_jump() {
        for bad in ["bastion:notaport", "", "a,,b", "[fd00::1", ":22"] {
            assert!(parse_proxy_jump(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[tokio::test]
    async fn resolve_rejects_flag_alias_before_spawning() {
        let err = resolve_ssh_config_with("-oProxyCommand=id", "/nonexistent/ssh")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid SSH config alias"));
    }

    #[tokio::test]
    async fn resolve_reads_real_ssh_g_output() {
        // `true` ignores args and exits 0 with empty stdout: exercises the spawn
        // + wait + parse path without needing a real ssh binary.
        let cfg = resolve_ssh_config_with("example", "/usr/bin/true")
            .await
            .unwrap();
        assert_eq!(cfg.port, 22);

        let err = resolve_ssh_config_with("example", "/usr/bin/false")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ssh -G example"));
    }
}
