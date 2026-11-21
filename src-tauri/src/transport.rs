use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, Sleep},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
type Handshake =
    Pin<Box<dyn Future<Output = Option<(TlsStream<LimitedStream>, SocketAddr)>> + Send>>;
const MAX_PER_IP: usize = 16;
const MAX_HANDSHAKES: usize = 64;
const MAX_HANDSHAKES_PER_IP: usize = 8;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

struct IpGuard {
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}
impl Drop for IpGuard {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap();
        if let Some(count) = counts.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}

pub struct LimitedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}
#[derive(Clone, Copy)]
pub struct Peer(pub SocketAddr);
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, LimitedListener>>
    for Peer
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, LimitedListener>) -> Self {
        Self(*stream.remote_addr())
    }
}
fn acquire_ip_slot(
    counts: &Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
    limit: usize,
) -> Option<IpGuard> {
    let mut map = counts.lock().unwrap();
    let entry = map.entry(ip).or_insert(0);
    if *entry >= limit {
        return None;
    }
    *entry += 1;
    Some(IpGuard {
        counts: counts.clone(),
        ip,
    })
}

impl LimitedListener {
    pub fn new(listener: TcpListener) -> Self {
        Self {
            listener,
            slots: Arc::new(Semaphore::new(32)),
            per_ip: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    fn acquire_ip(&self, ip: IpAddr) -> Option<IpGuard> {
        acquire_ip_slot(&self.per_ip, ip, MAX_PER_IP)
    }
}
const FIRST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct LimitedStream {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
    _ip: IpGuard,
    idle: Pin<Box<Sleep>>,
    write_idle: Pin<Box<Sleep>>,
}
impl axum::serve::Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((stream, address)) => {
                    // WebSocket acknowledgements are tiny and latency-sensitive.
                    // Do not let Nagle hold them behind an unacknowledged packet.
                    if stream.set_nodelay(true).is_err() {
                        continue;
                    }
                    if let (Ok(permit), Some(ip_guard)) = (
                        self.slots.clone().try_acquire_owned(),
                        self.acquire_ip(address.ip()),
                    ) {
                        let now = Instant::now();
                        return (
                            LimitedStream {
                                stream,
                                _permit: permit,
                                _ip: ip_guard,
                                idle: Box::pin(tokio::time::sleep_until(now + FIRST_READ_TIMEOUT)),
                                write_idle: Box::pin(tokio::time::sleep_until(
                                    now + WRITE_IDLE_TIMEOUT,
                                )),
                            },
                            address,
                        );
                    }
                    drop(stream);
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
impl AsyncRead for LimitedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.stream).poll_read(cx, buf) {
            Poll::Ready(result) => {
                self.idle.as_mut().reset(Instant::now() + READ_IDLE_TIMEOUT);
                Poll::Ready(result)
            }
            Poll::Pending => {
                if self.idle.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "connection idle timeout",
                    )))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}
impl AsyncWrite for LimitedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.stream).poll_write(cx, buf) {
            Poll::Ready(result) => {
                let now = Instant::now();
                self.write_idle.as_mut().reset(now + WRITE_IDLE_TIMEOUT);
                // Keep read-idle alive during downloads and slow responses where the peer has nothing more to send.
                if matches!(&result, Ok(written) if *written > 0) {
                    self.idle.as_mut().reset(now + READ_IDLE_TIMEOUT);
                }
                Poll::Ready(result)
            }
            Poll::Pending if self.write_idle.as_mut().poll(cx).is_ready() => Poll::Ready(Err(
                io::Error::new(io::ErrorKind::TimedOut, "connection write timeout"),
            )),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// TLS wrapper around the connection-limited TCP listener. TLS negotiation is
/// completed before Axum sees a request, so HTTP, pairing data and WebSockets
/// are encrypted on the wire.
pub struct TlsListener {
    listener: LimitedListener,
    acceptor: TlsAcceptor,
    handshakes: FuturesUnordered<Handshake>,
    pending_per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}
impl TlsListener {
    pub fn new(listener: TcpListener, config: Arc<rustls::ServerConfig>) -> Self {
        Self {
            listener: LimitedListener::new(listener),
            acceptor: TlsAcceptor::from(config),
            handshakes: FuturesUnordered::new(),
            pending_per_ip: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<LimitedStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            tokio::select! {
                result = self.handshakes.next(), if !self.handshakes.is_empty() => {
                    if let Some(Some(connection)) = result { return connection; }
                }
                (stream, address) = self.listener.accept(), if self.handshakes.len() < MAX_HANDSHAKES => {
                    let Some(_hs_guard) = acquire_ip_slot(
                        &self.pending_per_ip,
                        address.ip(),
                        MAX_HANDSHAKES_PER_IP,
                    ) else {
                        drop(stream);
                        continue;
                    };
                    let acceptor = self.acceptor.clone();
                    self.handshakes.push(Box::pin(async move {
                        let _guard = _hs_guard;
                        tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream))
                            .await.ok()?.ok().map(|stream| (stream, address))
                    }));
                }
            }
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TlsListener>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::serve::Listener;

    #[tokio::test]
    async fn accepted_connections_disable_nagle() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut listener = LimitedListener::new(listener);
        let (_, (accepted, _)) = tokio::join!(TcpStream::connect(address), listener.accept());
        assert!(accepted.stream.nodelay().unwrap());
    }

    #[tokio::test]
    async fn per_ip_connection_limit_is_enforced_and_released() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut listener = LimitedListener::new(listener);
        let mut clients = Vec::new();
        let mut accepted = Vec::new();
        for _ in 0..MAX_PER_IP {
            let (client, (stream, _)) =
                tokio::join!(TcpStream::connect(address), listener.accept());
            clients.push(client.unwrap());
            accepted.push(stream);
        }
        let extra = TcpStream::connect(address).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(80), listener.accept())
                .await
                .is_err()
        );
        drop(extra);
        accepted.pop();
        clients.pop();
        let (client, (stream, _)) = tokio::join!(TcpStream::connect(address), listener.accept());
        assert!(client.is_ok());
        drop(stream);
    }

    #[test]
    fn handshake_slot_limit_caps_per_ip_and_releases_on_drop() {
        let counts = Arc::new(Mutex::new(HashMap::new()));
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let mut guards = Vec::new();
        for _ in 0..MAX_HANDSHAKES_PER_IP {
            guards.push(acquire_ip_slot(&counts, ip, MAX_HANDSHAKES_PER_IP).unwrap());
        }
        assert!(acquire_ip_slot(&counts, ip, MAX_HANDSHAKES_PER_IP).is_none());
        let other: IpAddr = "127.0.0.2".parse().unwrap();
        let other_guard = acquire_ip_slot(&counts, other, MAX_HANDSHAKES_PER_IP);
        assert!(other_guard.is_some());
        drop(guards.pop());
        assert!(acquire_ip_slot(&counts, ip, MAX_HANDSHAKES_PER_IP).is_some());
    }
}
