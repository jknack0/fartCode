//! `RemoteHost` implementation over an SSH connection (E12-04).
//!
//! `fartcode-core` owns the trait (leaf-crate rule, ADR-0003); this is the
//! side that actually talks to the host: SFTP for filesystem facts, an exec
//! channel for git. Both ride the same [`SshClient`], so one profile connect
//! serves a whole create flow.

use std::sync::Arc;

use fartcode_core::projects::remote::{
    remote_command_line, RemoteEntry, RemoteFileKind, RemoteHost, RemoteOutput,
};
use fartcode_core::ssh_connections::{secrets, SshAuthType, SshConnection};
use fartcode_core::Error;
use tokio::sync::Mutex;

use crate::config::parse_proxy_jump;
use crate::sftp::{FileKind, RemoteSftp};
use crate::{AuthMethod, ConnectionParams, SshClient};

/// Where remote clones land when the profile does not override it.
pub const DEFAULT_REMOTE_PROJECTS_DIR: &str = "~/fartCode";

/// An SSH host viewed as a filesystem + command runner.
///
/// The SFTP session is bound to `/`: containment for remote projects is
/// enforced per operation by `fartcode_core::projects::remote::ensure_contained`
/// against the *project* root, which is not known when the session opens.
pub struct SshRemoteHost {
    client: Arc<SshClient>,
    /// One SFTP session, serialized — `RemoteSftp` needs `&mut`, and opening
    /// a session per call would cost a channel round trip each time.
    sftp: Mutex<RemoteSftp>,
}

impl SshRemoteHost {
    /// Wraps a live client, opening its SFTP session.
    pub async fn new(client: Arc<SshClient>) -> Result<Self, Error> {
        let sftp = client.sftp("/").await?;
        Ok(Self {
            client,
            sftp: Mutex::new(sftp),
        })
    }

    /// Connects a stored profile and wraps it.
    pub async fn connect(connection: &SshConnection) -> Result<Self, Error> {
        let client = Arc::new(connect_profile(connection).await?);
        Self::new(client).await
    }

    pub fn client(&self) -> &Arc<SshClient> {
        &self.client
    }
}

#[async_trait::async_trait]
impl RemoteHost for SshRemoteHost {
    async fn realpath(&self, path: &str) -> Result<String, Error> {
        self.sftp.lock().await.realpath(path).await
    }

    async fn list_dir(&self, path: &str, include_hidden: bool) -> Result<Vec<RemoteEntry>, Error> {
        let entries = self.sftp.lock().await.list(path, include_hidden).await?;
        Ok(entries
            .into_iter()
            .map(|e| RemoteEntry {
                path: format!("{}/{}", path.trim_end_matches('/'), e.name),
                name: e.name,
                kind: match e.kind {
                    FileKind::Dir => RemoteFileKind::Dir,
                    FileKind::Symlink => RemoteFileKind::Symlink,
                    FileKind::File => RemoteFileKind::File,
                },
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> Result<Option<RemoteFileKind>, Error> {
        Ok(self
            .sftp
            .lock()
            .await
            .stat(path)
            .await?
            .map(|e| match e.kind {
                FileKind::Dir => RemoteFileKind::Dir,
                FileKind::Symlink => RemoteFileKind::Symlink,
                FileKind::File => RemoteFileKind::File,
            }))
    }

    async fn mkdir_all(&self, path: &str) -> Result<(), Error> {
        self.sftp.lock().await.mkdir(path, true).await
    }

    async fn remove_dir_all(&self, path: &str) -> Result<(), Error> {
        let mut sftp = self.sftp.lock().await;
        // The trait contract is idempotent; `RemoteSftp::remove` treats a
        // missing path as an error, so absence is filtered here.
        if sftp.stat(path).await?.is_none() {
            return Ok(());
        }
        sftp.remove(path, true).await
    }

    async fn run(&self, argv: &[&str], cwd: Option<&str>) -> Result<RemoteOutput, Error> {
        let line = remote_command_line(argv, cwd);
        exec_collect(&self.client, &line).await
    }
}

/// Runs `command` and collects stdout, stderr, and the exit status.
///
/// `SshClient::run_command` only returns stdout — a failing `git rev-parse`
/// would look like an empty success, which is exactly the signal the remote
/// project flow keys on.
async fn exec_collect(client: &SshClient, command: &str) -> Result<RemoteOutput, Error> {
    let mut channel = client.exec(command).await?;
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let mut exit_code: Option<i32> = None;

    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
            russh::ChannelMsg::ExtendedData { ref data, ext } => {
                // ext 1 is stderr (RFC 4254 §5.2); anything else is not ours.
                if ext == 1 {
                    stderr.extend_from_slice(data);
                }
            }
            russh::ChannelMsg::ExitStatus { exit_status } => {
                exit_code = Some(exit_status as i32);
            }
            russh::ChannelMsg::ExitSignal {
                ref signal_name, ..
            } => {
                // Killed by a signal: no exit status will follow.
                stderr
                    .extend_from_slice(format!("terminated by signal {signal_name:?}").as_bytes());
                exit_code = Some(-1);
            }
            _ => {}
        }
    }

    Ok(RemoteOutput {
        // No status and no signal means the peer closed without reporting;
        // treat it as failure rather than silent success.
        exit_code: exit_code.unwrap_or(-1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Connects a stored profile: alias profiles go through `ssh -G` (E12-03),
/// manual ones through their own fields, and ProxyJump beats ProxyCommand —
/// the same precedence OpenSSH applies.
pub async fn connect_profile(connection: &SshConnection) -> Result<SshClient, Error> {
    let auth = auth_method(connection)?;
    let meta = connection.metadata.clone().unwrap_or_default();

    if let Some(alias) = meta.alias.as_deref() {
        return SshClient::connect_alias(alias, auth).await;
    }

    let params = ConnectionParams {
        host: connection.host.clone(),
        port: connection.port,
        username: connection.username.clone(),
        auth: auth.clone(),
        forward_agent: meta.forward_agent,
    };
    if let Some(spec) = meta.proxy_jump.as_deref() {
        let hops = parse_proxy_jump(spec)?;
        return SshClient::connect_via_jumps(&hops, auth, params).await;
    }
    if let Some(command) = meta.proxy_command.as_deref() {
        return SshClient::connect_via_proxy_command(params, command).await;
    }
    SshClient::connect(params).await
}

/// Profile → auth method. Secrets are read from the keyring here and nowhere
/// else; a missing key passphrase is "no passphrase", but a missing password
/// is an error (silently trying an empty one would lock accounts out).
fn auth_method(connection: &SshConnection) -> Result<AuthMethod, Error> {
    match connection.auth_type {
        SshAuthType::Agent => Ok(AuthMethod::Agent),
        SshAuthType::Password => Ok(AuthMethod::Password(secrets::load_secret(&connection.id)?)),
        SshAuthType::KeyFile => {
            let path = connection.private_key_path.as_deref().ok_or_else(|| {
                Error::SshAuth(format!(
                    "connection {} is key-file auth with no private_key_path",
                    connection.id
                ))
            })?;
            Ok(AuthMethod::KeyFile {
                path: std::path::PathBuf::from(shellexpand_home(path)),
                passphrase: secrets::load_secret(&connection.id).ok(),
            })
        }
    }
}

/// Expands a leading `~` against the local `$HOME` (key files are read on
/// this machine, not the remote).
fn shellexpand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{}/{rest}", home.trim_end_matches('/')),
            Err(_) => path.to_string(),
        },
        None => path.to_string(),
    }
}

/// The directory remote clones land in for this profile: the per-connection
/// override when set, else [`DEFAULT_REMOTE_PROJECTS_DIR`]. A leading `~` is
/// expanded on the **remote** host, so it needs a live connection.
pub async fn remote_projects_dir(
    host: &dyn RemoteHost,
    connection: &SshConnection,
) -> Result<String, Error> {
    let configured = connection
        .metadata
        .as_ref()
        .and_then(|m| m.projects_directory.clone())
        .unwrap_or_else(|| DEFAULT_REMOTE_PROJECTS_DIR.to_string());

    let Some(rest) = configured.strip_prefix('~') else {
        return Ok(configured);
    };
    let home = host.run(&["printf", "%s", "$HOME"], None).await?;
    // `$HOME` is not expanded inside single quotes, so ask the shell for it
    // explicitly instead of trusting the literal.
    let home = if home.ok() && !home.stdout_trimmed().is_empty() {
        home.stdout_trimmed().to_string()
    } else {
        let pwd = host.run(&["pwd"], None).await?;
        pwd.stdout_trimmed().to_string()
    };
    Ok(format!(
        "{}/{}",
        home.trim_end_matches('/'),
        rest.trim_start_matches('/')
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(auth: SshAuthType) -> SshConnection {
        SshConnection {
            id: "c1".into(),
            name: "box".into(),
            host: "example.test".into(),
            port: 22,
            username: "dev".into(),
            auth_type: auth,
            private_key_path: None,
            use_agent: true,
            metadata: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn key_file_auth_needs_a_path() {
        let err = auth_method(&profile(SshAuthType::KeyFile)).unwrap_err();
        assert!(err.to_string().contains("private_key_path"));
    }

    #[test]
    fn agent_auth_reads_no_secret() {
        assert!(matches!(
            auth_method(&profile(SshAuthType::Agent)).unwrap(),
            AuthMethod::Agent
        ));
    }
}
