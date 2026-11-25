use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{atomic::{AtomicBool,Ordering},Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{watch, OwnedSemaphorePermit, Semaphore},
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
#[derive(Clone)]
pub struct Peer(pub SocketAddr,pub(crate) Option<Arc<AtomicBool>>);
impl Peer {
    pub(crate) fn websocket_close_grace(&self) {if let Some(flag)=&self.1 {flag.store(true,Ordering::Release);}}
}
pub struct CancelListener<L> {
    inner: L,
    stop: watch::Receiver<bool>,
}
impl<L> CancelListener<L> {
    pub fn new(inner:L,stop:watch::Receiver<bool>)->Self {Self{inner,stop}}
}
impl<L:axum::serve::Listener> axum::serve::Listener for CancelListener<L> {
    type Io=CancelStream<L::Io>;
    type Addr=L::Addr;
    async fn accept(&mut self)->(Self::Io,Self::Addr) {
        let (stream,address)=self.inner.accept().await;
        (CancelStream::new(stream,self.stop.clone()),address)
    }
    fn local_addr(&self)->io::Result<Self::Addr> {self.inner.local_addr()}
}
impl<L:axum::serve::Listener<Addr=SocketAddr>> axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_,CancelListener<L>>> for Peer {
    fn connect_info(stream:axum::serve::IncomingStream<'_,CancelListener<L>>)->Self {Self(*stream.remote_addr(),Some(stream.io().websocket.clone()))}
}
pub struct CancelStream<S> {
    inner:S,
    stop:Pin<Box<dyn Future<Output=()>+Send>>,
    stopped:bool,
    websocket:Arc<AtomicBool>,
    close_deadline:Option<Pin<Box<Sleep>>>,
}
impl<S> CancelStream<S> {
    fn new(inner:S,mut signal:watch::Receiver<bool>)->Self {
        let stop=Box::pin(async move {while !*signal.borrow_and_update() {if signal.changed().await.is_err(){break;}}});
        Self{inner,stop,stopped:false,websocket:Arc::new(AtomicBool::new(false)),close_deadline:None}
    }
    fn stopped(&mut self,cx:&mut Context<'_>)->bool {
        if !self.stopped {self.stopped=self.stop.as_mut().poll(cx).is_ready();}
        self.stopped
    }
    fn write_stopped(&mut self,cx:&mut Context<'_>)->bool {
        if !self.stopped(cx) {return false;}
        if !self.websocket.load(Ordering::Acquire) {return true;}
        self.close_deadline.get_or_insert_with(||Box::pin(tokio::time::sleep(Duration::from_millis(500)))).as_mut().poll(cx).is_ready()
    }
}
fn connection_stopped()->io::Error {io::Error::new(io::ErrorKind::ConnectionAborted,"connection stopped")}
impl<S:AsyncRead+Unpin> AsyncRead for CancelStream<S> {
    fn poll_read(mut self:Pin<&mut Self>,cx:&mut Context<'_>,buf:&mut ReadBuf<'_>)->Poll<io::Result<()>> {
        if self.stopped(cx) {Poll::Ready(Err(connection_stopped()))} else {Pin::new(&mut self.inner).poll_read(cx,buf)}
    }
}
impl<S:AsyncWrite+Unpin> AsyncWrite for CancelStream<S> {
    fn poll_write(mut self:Pin<&mut Self>,cx:&mut Context<'_>,buf:&[u8])->Poll<io::Result<usize>> {
        if self.write_stopped(cx) {Poll::Ready(Err(connection_stopped()))} else {Pin::new(&mut self.inner).poll_write(cx,buf)}
    }
    fn poll_write_vectored(mut self:Pin<&mut Self>,cx:&mut Context<'_>,buf:&[io::IoSlice<'_>])->Poll<io::Result<usize>> {
        if self.write_stopped(cx) {Poll::Ready(Err(connection_stopped()))} else {Pin::new(&mut self.inner).poll_write_vectored(cx,buf)}
    }
    fn is_write_vectored(&self)->bool {self.inner.is_write_vectored()}
    fn poll_flush(mut self:Pin<&mut Self>,cx:&mut Context<'_>)->Poll<io::Result<()>> {
        if self.write_stopped(cx) {Poll::Ready(Err(connection_stopped()))} else {Pin::new(&mut self.inner).poll_flush(cx)}
    }
    fn poll_shutdown(mut self:Pin<&mut Self>,cx:&mut Context<'_>)->Poll<io::Result<()>> {
        if self.stopped(cx) {Poll::Ready(Ok(()))} else {Pin::new(&mut self.inner).poll_shutdown(cx)}
    }
}
impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, LimitedListener>>
    for Peer
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, LimitedListener>) -> Self {
        Self(*stream.remote_addr(),None)
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
        Self::with_slots(listener,Arc::new(Semaphore::new(32)))
    }
    pub(crate) fn with_slots(listener:TcpListener,slots:Arc<Semaphore>)->Self {
        Self {
            listener,
            slots,
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
        Self::with_slots(listener,config,Arc::new(Semaphore::new(32)))
    }
    pub(crate) fn with_slots(listener:TcpListener,config:Arc<rustls::ServerConfig>,slots:Arc<Semaphore>)->Self {
        Self {
            listener: LimitedListener::with_slots(listener,slots),
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
        Self(*stream.remote_addr(),None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::serve::Listener;
    #[tokio::test]
    async fn endpoint_cancel_wakes_pending_reads_and_rejects_buffered_io() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let (stop,signal)=watch::channel(false);let (inner,mut peer)=tokio::io::duplex(16);
        let mut stream=CancelStream::new(inner,signal);
        let reading=tokio::spawn(async move {let mut byte=[0];let result=stream.read(&mut byte).await;(stream,result)});
        tokio::task::yield_now().await;stop.send(true).unwrap();
        let (mut stream,result)=tokio::time::timeout(Duration::from_secs(1),reading).await.unwrap().unwrap();
        assert_eq!(result.unwrap_err().kind(),io::ErrorKind::ConnectionAborted);
        peer.write_all(b"queued").await.unwrap();
        assert!(stream.read(&mut [0]).await.is_err());assert!(stream.write_all(b"late").await.is_err());assert!(stream.flush().await.is_err());
    }
    #[tokio::test]
    async fn cancellation_is_per_endpoint_and_wakes_a_backpressured_write() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let (stop,signal)=watch::channel(false);let (inner,_peer)=tokio::io::duplex(1);let mut stream=CancelStream::new(inner,signal);
        let writing=tokio::spawn(async move {stream.write_all(b"blocked bytes").await});
        let (_other_stop,other_signal)=watch::channel(false);let (inner,mut peer)=tokio::io::duplex(16);let mut other=CancelStream::new(inner,other_signal);
        tokio::task::yield_now().await;stop.send(true).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(1),writing).await.unwrap().unwrap().is_err());
        other.write_all(b"ok").await.unwrap();let mut received=[0;2];peer.read_exact(&mut received).await.unwrap();assert_eq!(&received,b"ok");
    }
    #[tokio::test]
    async fn websocket_cancellation_allows_a_bounded_close_frame_but_no_further_reads() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let (stop,signal)=watch::channel(false);let (inner,mut peer)=tokio::io::duplex(16);let mut stream=CancelStream::new(inner,signal);
        stream.websocket.store(true,Ordering::Release);stop.send(true).unwrap();
        assert!(stream.read(&mut [0]).await.is_err());stream.write_all(b"close").await.unwrap();
        let mut bytes=[0;5];peer.read_exact(&mut bytes).await.unwrap();assert_eq!(&bytes,b"close");
        assert!(tokio::time::timeout(Duration::from_secs(1),stream.write_all(&[0;64])).await.unwrap().is_err());
    }
    #[tokio::test]
    async fn multiple_listeners_share_and_release_their_connection_budget() {
        let budget=Arc::new(Semaphore::new(2));let first=TcpListener::bind("127.0.0.1:0").await.unwrap();let second=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (first_address,second_address)=(first.local_addr().unwrap(),second.local_addr().unwrap());
        let mut first=LimitedListener::with_slots(first,budget.clone());let mut second=LimitedListener::with_slots(second,budget.clone());
        let (_client,(one,_))=tokio::join!(TcpStream::connect(first_address),first.accept());let (_client,(two,_))=tokio::join!(TcpStream::connect(second_address),second.accept());assert_eq!(budget.available_permits(),0);
        let rejected=TcpStream::connect(second_address).await.unwrap();assert!(tokio::time::timeout(Duration::from_millis(80),second.accept()).await.is_err());drop(rejected);drop(one);
        let (_client,(replacement,_))=tokio::join!(TcpStream::connect(second_address),second.accept());drop(two);drop(replacement);assert_eq!(budget.available_permits(),2);
    }

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
