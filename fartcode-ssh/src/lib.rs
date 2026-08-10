//! fartcode-ssh — SSH client layer (E12-01).
//!
//! Connect, authenticate (password / key / agent), open PTY channels,
//! and execute remote commands. Built on russh.

use std::path::PathBuf;
use std::sync::Arc;

use fartcode_core::Error;
use russh::client::{AuthResult, Config, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{Channel, Error as SshError};
use tokio::io::AsyncReadExt;
use tracing::{debug, info};

pub mod sftp;

// ── Auth method ──────────────────────────────────────────────

/// Authentication method for SSH connections.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Password authentication.
    Password(String),
    /// Public key from file path.
    KeyFile {
        path: PathBuf,
        passphrase: Option<String>,
    },
    /// Use SSH agent (SSH_AUTH_SOCK).
    Agent,
}

// ── Connection parameters ────────────────────────────────────

/// Parameters for establishing an SSH connection.
#[derive(Debug, Clone)]
pub struct ConnectionParams {
    /// Remote host (hostname or IP).
    pub host: String,
    /// SSH port (default 22).
    pub port: u16,
    /// Username for authentication.
    pub username: String,
    /// Authentication method.
    pub auth: AuthMethod,
}

// ── Russh handler ────────────────────────────────────────────

/// Handler for russh client events.
/// Accepts all server keys (dev mode; known_hosts in E12-03).
#[derive(Default)]
pub struct SshHandler;

impl russh::client::Handler for SshHandler {
    type Error = SshError;

    // ponytail: accept all keys, known_hosts in E12-03
    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// ── Client ───────────────────────────────────────────────────

/// Active SSH connection.
pub struct SshClient {
    /// Russh client handle.
    handle: Handle<SshHandler>,
    /// Parameters used for this connection (for reconnect/inspection).
    params: ConnectionParams,
}

impl SshClient {
    /// Connect to a remote host with the given parameters.
    pub async fn connect(params: ConnectionParams) -> Result<Self, Error> {
        info!(host = %params.host, port = params.port, user = %params.username, "connecting SSH");

        let config = Arc::new(Config::default());
        let handler = SshHandler;

        let mut handle =
            russh::client::connect(config, (params.host.clone(), params.port), handler)
                .await
                .map_err(|e| Error::SshConnection(format!("connection failed: {e}")))?;

        // Authenticate
        let authenticated = match &params.auth {
            AuthMethod::Password(pass) => {
                handle
                    .authenticate_password(params.username.clone(), pass.clone())
                    .await
            }
            AuthMethod::KeyFile { path, passphrase } => {
                authenticate_key_file(&mut handle, &params.username, path, passphrase.as_deref())
                    .await
            }
            AuthMethod::Agent => authenticate_agent(&mut handle, &params.username).await,
        }
        .map_err(|e| Error::SshAuth(format!("auth failed: {e}")))?;

        if !authenticated.success() {
            return Err(Error::SshAuth("server rejected authentication".into()));
        }

        info!(host = %params.host, "SSH connected");
        Ok(Self { handle, params })
    }

    /// Open a PTY (interactive shell) on the remote host.
    pub async fn pty(&self) -> Result<Channel<russh::client::Msg>, Error> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::SshChannel(format!("open session: {e}")))?;

        channel
            .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .map_err(|e| Error::SshChannel(format!("request pty: {e}")))?;

        channel
            .request_shell(true)
            .await
            .map_err(|e| Error::SshChannel(format!("request shell: {e}")))?;

        Ok(channel)
    }

    /// Open a PTY with custom dimensions.
    pub async fn pty_with_size(
        &self,
        col_width: u32,
        row_height: u32,
    ) -> Result<Channel<russh::client::Msg>, Error> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::SshChannel(format!("open session: {e}")))?;

        channel
            .request_pty(true, "xterm-256color", col_width, row_height, 0, 0, &[])
            .await
            .map_err(|e| Error::SshChannel(format!("request pty: {e}")))?;

        channel
            .request_shell(true)
            .await
            .map_err(|e| Error::SshChannel(format!("request shell: {e}")))?;

        Ok(channel)
    }

    /// Resize an existing PTY channel.
    pub async fn resize_pty(
        &self,
        channel: &Channel<russh::client::Msg>,
        col_width: u32,
        row_height: u32,
    ) -> Result<(), Error> {
        channel
            .window_change(col_width, row_height, 0, 0)
            .await
            .map_err(|e| Error::SshChannel(format!("resize pty: {e}")))?;
        Ok(())
    }

    /// Execute a remote command (non-interactive).
    pub async fn exec(&self, command: &str) -> Result<Channel<russh::client::Msg>, Error> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::SshChannel(format!("open session: {e}")))?;

        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| Error::SshChannel(format!("exec: {e}")))?;

        Ok(channel)
    }

    /// Run a command and collect stdout.
    pub async fn run_command(&self, command: &str) -> Result<String, Error> {
        let channel = self.exec(command).await?;
        let mut stream = channel.into_stream();
        let mut output = Vec::new();
        stream
            .read_to_end(&mut output)
            .await
            .map_err(|e| Error::SshChannel(format!("read output: {e}")))?;
        String::from_utf8(output).map_err(|e| Error::SshChannel(format!("invalid utf8: {e}")))
    }

    /// Open a direct TCP/IP channel (remote port forward).
    pub async fn forward_local(
        &self,
        bind_port: u32,
        destination: &str,
        dest_port: u32,
    ) -> Result<Channel<russh::client::Msg>, Error> {
        self.handle
            .channel_open_direct_tcpip(destination, dest_port, "127.0.0.1", bind_port)
            .await
            .map_err(|e| Error::SshChannel(format!("forward local: {e}")))
    }

    /// Disconnect from the remote host.
    pub async fn disconnect(&self) -> Result<(), Error> {
        info!(host = %self.params.host, "disconnecting SSH");
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "closing", "en-US")
            .await;
        Ok(())
    }

    /// Get the connection parameters (for reconnect/inspection).
    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }

    /// Open an SFTP session bound to the given workspace root.
    ///
    /// The `root` path is canonicalized on the remote host and used
    /// as the containment boundary for all subsequent file operations.
    pub async fn sftp(&self, root: &str) -> Result<sftp::RemoteSftp, Error> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::SshChannel(format!("open session: {e}")))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| Error::SshSftp(format!("subsystem request: {e}")))?;

        sftp::RemoteSftp::new(channel, root).await
    }
}

// ── Auth helpers ─────────────────────────────────────────────

async fn authenticate_key_file(
    handle: &mut Handle<SshHandler>,
    username: &str,
    path: &PathBuf,
    passphrase: Option<&str>,
) -> Result<AuthResult, SshError> {
    let key = load_secret_key(path, passphrase).map_err(|_| SshError::CouldNotReadKey)?;
    let pkw = PrivateKeyWithHashAlg::new(Arc::new(key), None);
    handle
        .authenticate_publickey(username.to_string(), pkw)
        .await
}

async fn authenticate_agent(
    handle: &mut Handle<SshHandler>,
    username: &str,
) -> Result<AuthResult, SshError> {
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|_| SshError::CouldNotReadKey)?;

    let keys = agent
        .request_identities()
        .await
        .map_err(|_| SshError::CouldNotReadKey)?;

    for key in keys {
        match handle
            .authenticate_publickey_with(username.to_string(), key.clone(), None, &mut agent)
            .await
        {
            Ok(result) if result.success() => return Ok(result),
            Ok(result) => debug!("agent key rejected: {:?}", result),
            Err(e) => debug!("agent auth error: {e}"),
        }
    }

    Err(SshError::NoAuthMethod)
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_method_is_clone() {
        let auth = AuthMethod::Password("test".into());
        let _copy = auth.clone();
    }

    #[test]
    fn keyfile_auth_clone() {
        let auth = AuthMethod::KeyFile {
            path: PathBuf::from("~/.ssh/id_ed25519"),
            passphrase: None,
        };
        let _copy = auth.clone();
    }

    #[test]
    fn connection_params_debug() {
        let params = ConnectionParams {
            host: "localhost".into(),
            port: 22,
            username: "test".into(),
            auth: AuthMethod::Agent,
        };
        let _ = format!("{params:?}");
    }

    #[test]
    fn default_handler() {
        let _handler = SshHandler;
    }
}
