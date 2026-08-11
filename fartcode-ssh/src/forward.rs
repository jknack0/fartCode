//! SSH port-forward tunnels (E12-09, shared with E6-04).
//!
//! A tunnel binds a local loopback `TcpListener` and forwards every accepted
//! socket through a fresh direct-tcpip channel on the pooled SSH connection.
//! Reference: emdash `core/port-forwards/port-forward-tunnel.ts`.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fartcode_core::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::SshClient;

/// Local tunnels only ever bind loopback — a forward is for THIS machine,
/// not a LAN-exposed proxy to the remote host.
const LOCAL_BIND_HOST: &str = "127.0.0.1";

/// A dev server may listen on the IPv4 loopback, the IPv6 loopback, or both.
/// A process started on the default `localhost` often binds only `[::1]`
/// (Node >= 17 resolves the IPv6 loopback first), so a single hardcoded
/// `127.0.0.1` target misses it. Try both families per connection, in order,
/// and forward through whichever one the remote accepts.
const REMOTE_TARGET_HOSTS: [&str; 2] = ["127.0.0.1", "::1"];

/// True when the server refused a direct-tcpip channel because IT could not
/// connect to the requested destination (RFC 4254 `SSH_OPEN_CONNECT_FAILED`)
/// — the only refusal worth retrying on the other loopback family. Other
/// reasons (administratively prohibited, resource shortage, dropped session)
/// would not be fixed by a retry.
///
/// russh renders the reason as `Failed to open channel (ConnectFailed)`, and
/// `SshClient::forward_local` wraps that string in `Error::SshChannel`.
pub fn is_connect_failed(error: &Error) -> bool {
    error.to_string().to_lowercase().contains("connectfailed")
}

/// A bidirectional byte stream to the remote destination.
pub type TunnelStream = Pin<Box<dyn AsyncReadWrite + Send>>;

/// Object-safe `AsyncRead + AsyncWrite` bound for boxed tunnel streams.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> AsyncReadWrite for T {}

/// Opens direct-tcpip channels for a tunnel. The app layer implements this
/// over the pooled connection registry so every dial gets the live (possibly
/// rehydrated) session and channel failures feed degradation tracking.
#[async_trait]
pub trait TunnelDialer: Send + Sync {
    /// Open a stream to `remote_host:remote_port` as seen from the SSH host.
    async fn dial(&self, remote_host: &str, remote_port: u16) -> Result<TunnelStream, Error>;

    /// Cheap liveness check consulted before dialing; a dead connection
    /// drops the local socket instead of queueing a doomed channel open.
    fn is_connected(&self) -> bool {
        true
    }
}

#[async_trait]
impl TunnelDialer for SshClient {
    async fn dial(&self, remote_host: &str, remote_port: u16) -> Result<TunnelStream, Error> {
        let channel = self
            .forward_local(0, remote_host, u32::from(remote_port))
            .await?;
        Ok(Box::pin(channel.into_stream()))
    }

    fn is_connected(&self) -> bool {
        !self.is_closed()
    }
}

/// Options for [`open_tunnel`].
pub struct OpenTunnelOptions {
    /// Destination port on the remote host's loopback.
    pub remote_port: u16,
    /// Preferred local port; `None` (or busy) means ephemeral.
    pub preferred_local_port: Option<u16>,
}

/// A live local listener forwarding to a remote port. Dropping the tunnel
/// WITHOUT calling [`PortForwardTunnel::close`] aborts the accept loop but
/// may leave in-flight sockets to finish on their own; `close` kills both.
pub struct PortForwardTunnel {
    local_port: u16,
    accept_task: JoinHandle<()>,
    socket_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl PortForwardTunnel {
    /// The port the listener actually bound (ephemeral fallback included).
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Stops accepting and destroys every live forwarded socket.
    pub fn close(&self) {
        self.accept_task.abort();
        let tasks = std::mem::take(&mut *self.socket_tasks.lock().expect("socket task lock"));
        for task in tasks {
            task.abort();
        }
    }
}

impl Drop for PortForwardTunnel {
    fn drop(&mut self) {
        self.close();
    }
}

/// Binds the local listener and starts the accept loop.
///
/// A busy `preferred_local_port` falls back to an ephemeral port — the
/// caller reads the real port off the returned tunnel. Any other bind error
/// is surfaced.
pub async fn open_tunnel(
    dialer: Arc<dyn TunnelDialer>,
    options: OpenTunnelOptions,
) -> Result<PortForwardTunnel, Error> {
    let listener = match options.preferred_local_port {
        Some(port) => match TcpListener::bind((LOCAL_BIND_HOST, port)).await {
            Ok(listener) => listener,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => bind_ephemeral().await?,
            Err(e) => return Err(Error::SshChannel(format!("port forward bind: {e}"))),
        },
        None => bind_ephemeral().await?,
    };
    let local_port = listener
        .local_addr()
        .map_err(|e| Error::SshChannel(format!("port forward local addr: {e}")))?
        .port();

    let socket_tasks: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    let tasks = socket_tasks.clone();
    let remote_port = options.remote_port;
    let accept_task = tokio::spawn(async move {
        loop {
            let (socket, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(e) => {
                    debug!(error = %e, "port forward accept failed");
                    continue;
                }
            };
            let dialer = dialer.clone();
            let handle = tokio::spawn(async move {
                forward_socket(socket, dialer, remote_port).await;
            });
            let mut tasks = tasks.lock().expect("socket task lock");
            tasks.retain(|t| !t.is_finished());
            tasks.push(handle);
        }
    });

    Ok(PortForwardTunnel {
        local_port,
        accept_task,
        socket_tasks,
    })
}

async fn bind_ephemeral() -> Result<TcpListener, Error> {
    TcpListener::bind((LOCAL_BIND_HOST, 0))
        .await
        .map_err(|e| Error::SshChannel(format!("port forward bind: {e}")))
}

/// Dials the remote loopback (IPv4 then IPv6 on connect-failure) and pumps
/// bytes both ways until either side closes.
async fn forward_socket(mut socket: TcpStream, dialer: Arc<dyn TunnelDialer>, remote_port: u16) {
    if !dialer.is_connected() {
        return;
    }
    let mut first_error: Option<Error> = None;
    let mut stream: Option<TunnelStream> = None;
    for remote_host in REMOTE_TARGET_HOSTS {
        match dialer.dial(remote_host, remote_port).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => {
                // Only fall back to the next loopback family when the remote
                // could not connect to this one; any other failure would not
                // be fixed by a retry, so surface it instead of masking it
                // behind a second dial.
                let retry = is_connect_failed(&e);
                if first_error.is_none() {
                    first_error = Some(e);
                }
                if !retry {
                    break;
                }
            }
        }
    }
    let Some(mut stream) = stream else {
        if let Some(e) = first_error {
            warn!(remote_port, error = %e, "port forward dial failed");
        }
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Dialer that connects to a real local TCP "remote", refusing chosen
    /// loopback families the way a one-family dev server would.
    struct FakeDialer {
        target_port: u16,
        refuse_hosts: Vec<&'static str>,
        connected: bool,
        dials: Mutex<Vec<String>>,
    }

    impl FakeDialer {
        fn new(target_port: u16) -> Self {
            Self {
                target_port,
                refuse_hosts: Vec::new(),
                connected: true,
                dials: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TunnelDialer for FakeDialer {
        async fn dial(&self, remote_host: &str, _remote_port: u16) -> Result<TunnelStream, Error> {
            self.dials.lock().unwrap().push(remote_host.to_string());
            if self.refuse_hosts.contains(&remote_host) {
                return Err(Error::SshChannel(
                    "forward local: Failed to open channel (ConnectFailed)".into(),
                ));
            }
            let stream = TcpStream::connect(("127.0.0.1", self.target_port))
                .await
                .map_err(|e| Error::SshChannel(format!("fake dial: {e}")))?;
            Ok(Box::pin(stream))
        }

        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    /// Echo server on an ephemeral port; returns the port.
    async fn spawn_echo() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(a) => a,
                    Err(_) => break,
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if socket.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        port
    }

    async fn roundtrip(port: u16) -> Vec<u8> {
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        buf.to_vec()
    }

    #[tokio::test]
    async fn forwards_bytes_end_to_end() {
        let echo = spawn_echo().await;
        let dialer = Arc::new(FakeDialer::new(echo));
        let tunnel = open_tunnel(
            dialer,
            OpenTunnelOptions {
                remote_port: echo,
                preferred_local_port: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(roundtrip(tunnel.local_port()).await, b"ping");
        tunnel.close();
    }

    #[tokio::test]
    async fn falls_back_to_ipv6_loopback_on_connect_failed() {
        let echo = spawn_echo().await;
        let mut fake = FakeDialer::new(echo);
        fake.refuse_hosts = vec!["127.0.0.1"];
        let dialer = Arc::new(fake);
        let tunnel = open_tunnel(
            dialer.clone(),
            OpenTunnelOptions {
                remote_port: echo,
                preferred_local_port: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(roundtrip(tunnel.local_port()).await, b"ping");
        assert_eq!(*dialer.dials.lock().unwrap(), vec!["127.0.0.1", "::1"]);
        tunnel.close();
    }

    #[tokio::test]
    async fn busy_preferred_port_falls_back_to_ephemeral() {
        let echo = spawn_echo().await;
        // Occupy a port so the preferred bind fails with AddrInUse.
        let occupied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let busy_port = occupied.local_addr().unwrap().port();
        let dialer = Arc::new(FakeDialer::new(echo));
        let tunnel = open_tunnel(
            dialer,
            OpenTunnelOptions {
                remote_port: echo,
                preferred_local_port: Some(busy_port),
            },
        )
        .await
        .unwrap();
        assert_ne!(tunnel.local_port(), busy_port);
        assert_eq!(roundtrip(tunnel.local_port()).await, b"ping");
        tunnel.close();
    }

    #[tokio::test]
    async fn close_stops_listener_and_sockets() {
        let echo = spawn_echo().await;
        let dialer = Arc::new(FakeDialer::new(echo));
        let tunnel = open_tunnel(
            dialer,
            OpenTunnelOptions {
                remote_port: echo,
                preferred_local_port: None,
            },
        )
        .await
        .unwrap();
        let port = tunnel.local_port();
        assert_eq!(roundtrip(port).await, b"ping");
        tunnel.close();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }

    #[tokio::test]
    async fn disconnected_dialer_drops_socket_without_dialing() {
        let echo = spawn_echo().await;
        let mut fake = FakeDialer::new(echo);
        fake.connected = false;
        let dialer = Arc::new(fake);
        let tunnel = open_tunnel(
            dialer.clone(),
            OpenTunnelOptions {
                remote_port: echo,
                preferred_local_port: None,
            },
        )
        .await
        .unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", tunnel.local_port()))
            .await
            .unwrap();
        let mut buf = [0u8; 1];
        // Peer closes without data.
        assert_eq!(client.read(&mut buf).await.unwrap(), 0);
        assert!(dialer.dials.lock().unwrap().is_empty());
        tunnel.close();
    }
}
