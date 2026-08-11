//! BYOI over SSH (E12-07): turning a provision descriptor into a connection,
//! and running the project's scripts on a remote host.
//!
//! The contract itself lives in `fartcode_core::tasks::byoi` — core is the
//! leaf crate, so it owns the shapes and this module owns the transport.

use std::sync::Arc;
use std::time::Duration;

use fartcode_core::projects::remote::{RemoteHost, RemoteOutput};
use fartcode_core::tasks::byoi::{script_command_line, ProvisionOutput, ScriptRunner};
use fartcode_core::Error;

use crate::host::SshRemoteHost;
use crate::{AuthMethod, ConnectionParams};

/// Splits a `user@host` descriptor field. A leading or trailing `@` is part
/// of the host, not a username — `@box` and `box@` are hostnames as written.
fn split_user_host(host: &str) -> (String, Option<String>) {
    match host.rfind('@') {
        Some(at) if at > 0 && at < host.len() - 1 => {
            (host[at + 1..].to_string(), Some(host[..at].to_string()))
        }
        _ => (host.to_string(), None),
    }
}

/// Connection parameters for a machine a provision script just described
/// (E12-07 AC7/AC8).
///
/// `ssh_auth_sock` is passed in rather than read here so the resolution is
/// testable and so a caller can decide what "the agent" means (the app reads
/// `SSH_AUTH_SOCK`).
///
/// Refuses rather than degrades in two places: agent forwarding without an
/// agent, and no usable credential at all. Both would otherwise fail later as
/// an opaque auth error against a machine the user is already paying for.
pub fn connect_params(
    output: &ProvisionOutput,
    ssh_auth_sock: Option<&str>,
) -> Result<ConnectionParams, Error> {
    let (host, host_username) = split_user_host(&output.host);
    let username = output
        .username
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(String::from)
        .or(host_username)
        .or_else(|| std::env::var("USER").ok())
        .ok_or_else(|| {
            Error::InvalidTaskInput(
                "provision output has no username, and no $USER to fall back on".into(),
            )
        })?;

    let password = output
        .password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(String::from);
    let forward_agent = output.forward_agent == Some(true);
    let has_agent = ssh_auth_sock.is_some_and(|s| !s.is_empty());

    if forward_agent && !has_agent {
        return Err(Error::InvalidTaskInput(
            "provision output asked for agent forwarding, but no SSH agent socket is available"
                .into(),
        ));
    }

    // Password wins when present: it is the credential the provisioner minted
    // for this machine. The agent is the fallback — and the only option when
    // the descriptor carries no secret at all.
    let auth = match password {
        Some(password) => AuthMethod::Password(password),
        None if has_agent => AuthMethod::Agent,
        None => {
            return Err(Error::InvalidTaskInput(
                "provision output has no password and no SSH agent is available".into(),
            ))
        }
    };

    Ok(ConnectionParams {
        host,
        port: output.port.unwrap_or(22),
        username,
        auth,
        forward_agent,
    })
}

/// Runs provision/terminate scripts on an SSH host — the implementation a
/// REMOTE project uses, where the scripts belong on the machine holding the
/// repository, not on this laptop.
pub struct SshScriptRunner {
    host: Arc<SshRemoteHost>,
    /// Project directory on the host; scripts run there.
    cwd: Option<String>,
}

impl SshScriptRunner {
    pub fn new(host: Arc<SshRemoteHost>, cwd: Option<String>) -> Self {
        Self { host, cwd }
    }
}

#[async_trait::async_trait]
impl ScriptRunner for SshScriptRunner {
    async fn run_script(
        &self,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<RemoteOutput, Error> {
        // A remote shell has no `Command::envs`, so the assignments are part
        // of the line — quoted by `script_command_line`, never interpolated.
        let line = script_command_line(command, env)?;
        let argv = ["/bin/sh", "-c", line.as_str()];
        let run = self.host.run(&argv, self.cwd.as_deref());
        match tokio::time::timeout(timeout, run).await {
            Ok(result) => result,
            Err(_) => Err(Error::Internal(format!(
                "script timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(json: &str) -> ProvisionOutput {
        fartcode_core::tasks::byoi::parse_provision_output(json).unwrap()
    }

    #[test]
    fn splits_user_qualified_hosts() {
        assert_eq!(
            split_user_host("ci@10.0.0.4"),
            ("10.0.0.4".to_string(), Some("ci".to_string()))
        );
        assert_eq!(split_user_host("box"), ("box".to_string(), None));
        // Degenerate forms stay hostnames rather than becoming empty users.
        assert_eq!(split_user_host("@box"), ("@box".to_string(), None));
        assert_eq!(split_user_host("box@"), ("box@".to_string(), None));
    }

    #[test]
    fn explicit_username_beats_the_host_prefix() {
        let params = connect_params(
            &output(r#"{"id":"1","host":"ci@box","username":"deploy","password":"p"}"#),
            None,
        )
        .unwrap();
        assert_eq!(params.username, "deploy");
        assert_eq!(params.host, "box");
        assert_eq!(params.port, 22);
        assert!(matches!(params.auth, AuthMethod::Password(_)));
        assert!(!params.forward_agent);
    }

    #[test]
    fn port_and_agent_defaults() {
        let params = connect_params(
            &output(r#"{"id":"1","host":"ci@box","port":2222}"#),
            Some("/tmp/agent.sock"),
        )
        .unwrap();
        assert_eq!(params.port, 2222);
        assert_eq!(params.username, "ci");
        assert!(matches!(params.auth, AuthMethod::Agent));
    }

    /// AC8: forwarding without an agent is a refusal, not a quiet downgrade.
    #[test]
    fn forward_agent_without_a_socket_is_refused() {
        let descriptor = output(r#"{"id":"1","host":"ci@box","password":"p","forwardAgent":true}"#);
        let error = connect_params(&descriptor, None).unwrap_err().to_string();
        assert!(error.contains("agent forwarding"), "{error}");

        let params = connect_params(&descriptor, Some("/tmp/agent.sock")).unwrap();
        assert!(params.forward_agent);
    }

    #[test]
    fn no_credential_at_all_is_refused() {
        let error = connect_params(&output(r#"{"id":"1","host":"ci@box"}"#), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no password"), "{error}");
    }

    #[test]
    fn params_debug_never_prints_the_password() {
        let params = connect_params(
            &output(r#"{"id":"1","host":"ci@box","password":"hunter2"}"#),
            None,
        )
        .unwrap();
        assert!(!format!("{params:?}").contains("hunter2"));
    }
}
