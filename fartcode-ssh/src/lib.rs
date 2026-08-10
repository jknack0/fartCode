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

pub mod config;
pub mod sftp;

// ── Auth method ──────────────────────────────────────────────

/// Authentication method for SSH connections.
#[derive(Clone)]
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

/// Hand-written so a stray `{:?}` (or a `ConnectionParams` dump in a log line)
/// can never print a password or passphrase.
impl std::fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Password(<redacted>)"),
            Self::KeyFile { path, passphrase } => f
                .debug_struct("KeyFile")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
            Self::Agent => f.write_str("Agent"),
        }
    }
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
    /// Forward the local SSH agent to channels opened on this connection
    /// (`ForwardAgent`). Off unless the profile asked for it — agent forwarding
    /// hands the remote host use of your keys.
    pub forward_agent: bool,
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
    /// ProxyJump hops kept alive: each one carries the tunnel for the next.
    via: Vec<SshClient>,
    /// ProxyCommand child, killed on drop.
    proxy: Option<tokio::process::Child>,
}

impl std::fmt::Debug for SshClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshClient")
            .field("host", &self.params.host)
            .field("port", &self.params.port)
            .field("username", &self.params.username)
            .field("forward_agent", &self.params.forward_agent)
            .field("jump_hops", &self.via.len())
            .field("proxy_command", &self.proxy.is_some())
            .finish()
    }
}

impl SshClient {
    /// Connect to a remote host with the given parameters.
    pub async fn connect(params: ConnectionParams) -> Result<Self, Error> {
        info!(host = %params.host, port = params.port, user = %params.username, "connecting SSH");

        let config = Arc::new(Config::default());
        let handler = SshHandler;

        let handle = russh::client::connect(config, (params.host.clone(), params.port), handler)
            .await
            .map_err(|e| Error::SshConnection(format!("connection failed: {e}")))?;

        Self::finish_auth(handle, params).await
    }

    /// Connect using a `~/.ssh/config` alias: resolve with `ssh -G`, then honor
    /// `ProxyJump` (which overrides `ProxyCommand`, as OpenSSH does) or
    /// `ProxyCommand`, falling back to a direct connection.
    pub async fn connect_alias(alias: &str, auth: AuthMethod) -> Result<Self, Error> {
        let cfg = config::resolve_ssh_config(alias).await?;
        let params = cfg.to_params(auth.clone())?;

        if let Some(spec) = cfg.proxy_jump.as_deref() {
            let hops = config::parse_proxy_jump(spec)?;
            return Self::connect_via_jumps(&hops, auth, params).await;
        }
        if let Some(command) = cfg.proxy_command.as_deref() {
            return Self::connect_via_proxy_command(params, command).await;
        }
        Self::connect(params).await
    }

    /// Connect over an already-established byte stream (a jump host's
    /// direct-tcpip channel, a `ProxyCommand` pipe, a test duplex).
    pub async fn connect_over<S>(params: ConnectionParams, stream: S) -> Result<Self, Error>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let handle = russh::client::connect_stream(Arc::new(Config::default()), stream, SshHandler)
            .await
            .map_err(|e| Error::SshConnection(format!("connection failed: {e}")))?;
        Self::finish_auth(handle, params).await
    }

    /// Connect to `target` *through* this connection, using a direct-tcpip
    /// channel as the transport.
    ///
    /// This is what `ProxyJump` means: the SSH handshake and authentication
    /// happen end-to-end with the target, so the jump host never sees the
    /// target's credentials (unlike shelling out to `ssh` on the jump host).
    pub async fn jump_to(&self, target: ConnectionParams) -> Result<Self, Error> {
        let channel = self
            .handle
            .channel_open_direct_tcpip(target.host.clone(), u32::from(target.port), "127.0.0.1", 0)
            .await
            .map_err(|e| {
                Error::SshChannel(format!(
                    "jump to {}:{} via {}: {e}",
                    target.host, target.port, self.params.host
                ))
            })?;
        Self::connect_over(target, channel.into_stream()).await
    }

    /// Connect to `target` through a `ProxyJump` chain, left to right.
    ///
    /// Every hop stays open for the life of the returned client; dropping an
    /// intermediate would collapse the tunnel underneath it.
    pub async fn connect_via_jumps(
        hops: &[config::JumpHop],
        hop_auth: AuthMethod,
        target: ConnectionParams,
    ) -> Result<Self, Error> {
        let mut chain: Vec<SshClient> = Vec::new();
        let mut current: Option<SshClient> = None;

        for hop in hops {
            let hop_params = ConnectionParams {
                host: hop.host.clone(),
                port: hop.port,
                // A hop without an explicit user inherits the target's, the
                // same defaulting `ssh -J` applies.
                username: hop.user.clone().unwrap_or_else(|| target.username.clone()),
                auth: hop_auth.clone(),
                forward_agent: false,
            };
            let next = match current.take() {
                None => SshClient::connect(hop_params).await?,
                Some(prev) => {
                    let next = prev.jump_to(hop_params).await?;
                    chain.push(prev);
                    next
                }
            };
            current = Some(next);
        }

        match current {
            None => SshClient::connect(target).await,
            Some(last) => {
                let mut client = last.jump_to(target).await?;
                chain.push(last);
                client.via = chain;
                Ok(client)
            }
        }
    }

    /// Connect through a `ProxyCommand`.
    ///
    /// Run by `/bin/sh -c` with its stdio as the SSH transport, matching
    /// OpenSSH. `ssh -G` has already expanded `%h`/`%p`, so the string is
    /// passed verbatim — re-quoting a command line the user wrote as shell
    /// syntax would break it. The child is killed when the client drops.
    pub async fn connect_via_proxy_command(
        params: ConnectionParams,
        proxy_command: &str,
    ) -> Result<Self, Error> {
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(proxy_command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::SshConnection(format!("spawn ProxyCommand: {e}")))?;

        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(Error::SshConnection(
                "ProxyCommand stdio unavailable".into(),
            ));
        };

        let mut client = Self::connect_over(params, tokio::io::join(stdout, stdin)).await?;
        client.proxy = Some(child);
        Ok(client)
    }

    /// Authenticate an established transport and wrap it up as a client.
    async fn finish_auth(
        mut handle: Handle<SshHandler>,
        params: ConnectionParams,
    ) -> Result<Self, Error> {
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
        Ok(Self {
            handle,
            params,
            via: Vec::new(),
            proxy: None,
        })
    }

    /// Request agent forwarding on a channel when the profile enables it.
    async fn maybe_forward_agent(
        &self,
        channel: &Channel<russh::client::Msg>,
    ) -> Result<(), Error> {
        if !self.params.forward_agent {
            return Ok(());
        }
        channel
            .agent_forward(true)
            .await
            .map_err(|e| Error::SshChannel(format!("agent forward: {e}")))
    }

    /// Open a PTY (interactive shell) on the remote host.
    pub async fn pty(&self) -> Result<Channel<russh::client::Msg>, Error> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::SshChannel(format!("open session: {e}")))?;

        self.maybe_forward_agent(&channel).await?;

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

        self.maybe_forward_agent(&channel).await?;

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

        self.maybe_forward_agent(&channel).await?;

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
            forward_agent: false,
            host: "localhost".into(),
            port: 22,
            username: "test".into(),
            auth: AuthMethod::Agent,
        };
        let _ = format!("{params:?}");
    }

    #[test]
    fn debug_never_prints_secrets() {
        let params = ConnectionParams {
            host: "localhost".into(),
            port: 22,
            username: "test".into(),
            auth: AuthMethod::Password("hunter2".into()),
            forward_agent: false,
        };
        let dump = format!("{params:?}");
        assert!(!dump.contains("hunter2"), "password leaked: {dump}");
        assert!(dump.contains("<redacted>"));

        let keyfile = AuthMethod::KeyFile {
            path: PathBuf::from("/k"),
            passphrase: Some("opensesame".into()),
        };
        let dump = format!("{keyfile:?}");
        assert!(!dump.contains("opensesame"), "passphrase leaked: {dump}");
    }

    #[test]
    fn default_handler() {
        let _handler = SshHandler;
    }

    #[tokio::test]
    async fn proxy_command_failure_surfaces_as_connection_error() {
        let params = ConnectionParams {
            host: "target.internal".into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Agent,
            forward_agent: false,
        };
        // The proxy dies immediately, so the SSH handshake sees EOF.
        let err = SshClient::connect_via_proxy_command(params, "exit 3")
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::SshConnection(_)),
            "expected SshConnection, got {err:?}"
        );
    }

    #[tokio::test]
    async fn connect_over_dead_stream_fails_without_hanging() {
        let (client_side, server_side) = tokio::io::duplex(64);
        drop(server_side);
        let params = ConnectionParams {
            host: "target.internal".into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Agent,
            forward_agent: false,
        };
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            SshClient::connect_over(params, client_side),
        )
        .await
        .expect("connect_over must not hang on a dead transport")
        .unwrap_err();
        assert!(matches!(err, Error::SshConnection(_)));
    }
}
