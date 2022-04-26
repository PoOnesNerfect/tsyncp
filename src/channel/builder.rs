use super::Channel;
use crate::util::{frame_codec::VariedLengthDelimitedCodec, Framed};
use errors::*;
use futures::Future;
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::io;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpSocket, TcpStream};
use tokio::task::JoinError;

pub type Result<T, E = ChannelBuilderError> = std::result::Result<T, E>;

pub(crate) fn new<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    addr: A,
    listening: bool,
) -> ChannelBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = Result<Channel<T, E>>>,
> {
    let max_retries = None;
    let retry_sleep_duration = Duration::from_millis(1000);
    let filter = |_| true;
    let tcp_settings = TcpSettings::default();

    ChannelBuilderFuture {
        addr: addr.clone(),
        listening,
        max_retries,
        retry_sleep_duration,
        tcp_settings,
        fut: build_tcp_stream::<A, T, E, _>(
            addr,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            filter.clone(),
        ),
        filter,
        _phantom: PhantomData,
    }
}

#[derive(Debug)]
#[pin_project]
pub struct ChannelBuilderFuture<A, T, E, Filter, Fut, S = TcpStream> {
    addr: A,
    listening: bool,
    max_retries: Option<usize>,
    retry_sleep_duration: Duration,
    tcp_settings: TcpSettings,
    filter: Filter,
    #[pin]
    fut: Fut,
    _phantom: PhantomData<(T, E, S)>,
}

#[derive(Debug, Default, Clone, Copy)]
struct TcpSettings {
    reuseaddr: Option<bool>,
    reuseport: Option<bool>,
    linger: Option<Option<Duration>>,
    recv_buffer_size: Option<u32>,
    send_buffer_size: Option<u32>,
    nodelay: Option<bool>,
    ttl: Option<u32>,
}

impl<A, T, E, Filter, Fut> ChannelBuilderFuture<A, T, E, Filter, Fut>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        mut self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.max_retries.replace(max_retries);
        self.retry_sleep_duration = retry_sleep_duration;

        self.refresh()
    }

    pub fn filter<Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> ChannelBuilderFuture<A, T, E, Filter2, impl Future<Output = Result<Channel<T, E>>>> {
        let ChannelBuilderFuture {
            addr,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            ..
        } = self;

        ChannelBuilderFuture {
            addr: addr.clone(),
            listening,
            max_retries,
            retry_sleep_duration,
            fut: build_tcp_stream::<A, T, E, Filter2>(
                addr,
                listening,
                max_retries,
                retry_sleep_duration,
                tcp_settings.clone(),
                filter.clone(),
            ),
            tcp_settings,
            filter,
            _phantom: PhantomData,
        }
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.reuseaddr.replace(reuseaddr);

        self.refresh()
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    pub fn set_tcp_reuseport(
        mut self,
        reuseport: bool,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.reuseport.replace(reuseport);

        self.refresh()
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.linger.replace(dur);

        self.refresh()
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.nodelay.replace(nodelay);

        self.refresh()
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.ttl.replace(ttl);

        self.refresh()
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.recv_buffer_size.replace(size);

        self.refresh()
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        self.tcp_settings.send_buffer_size.replace(size);

        self.refresh()
    }

    fn refresh(
        self,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E>>>> {
        let ChannelBuilderFuture {
            addr,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            filter,
            ..
        } = self;

        ChannelBuilderFuture {
            addr: addr.clone(),
            listening,
            max_retries,
            retry_sleep_duration,
            fut: build_tcp_stream::<A, T, E, Filter>(
                addr,
                listening,
                max_retries,
                retry_sleep_duration,
                tcp_settings.clone(),
                filter.clone(),
            ),
            tcp_settings,
            filter,
            _phantom: PhantomData,
        }
    }
}

impl<A, T, E, Filter, Fut: Future<Output = Result<Channel<T, E, S>>>, S> Future
    for ChannelBuilderFuture<A, T, E, Filter, Fut, S>
{
    type Output = Result<Channel<T, E, S>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx)
    }
}

async fn build_tcp_stream<
    A: 'static + Send + ToSocketAddrs,
    T,
    E,
    Filter: Fn(SocketAddr) -> bool,
>(
    addr: A,
    listening: bool,
    max_retries: Option<usize>,
    retry_sleep_duration: Duration,
    tcp_settings: TcpSettings,
    filter: Filter,
) -> Result<Channel<T, E>> {
    let addr = tokio::task::spawn_blocking(move || {
        addr.to_socket_addrs()?.next().ok_or(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("Given input could not be parsed to socket addr"),
        ))
    })
    .await
    .context(SpawnJoinSnafu)?
    .context(ToSocketAddrsSnafu)?;

    let socket = get_socket(&addr, &tcp_settings)?;

    let (stream, local_addr, peer_addr) = if listening {
        socket.bind(addr).context(BindingSnafu { addr })?;

        let listener = socket.listen(1024).context(ListeningSnafu { addr })?;

        let local_addr = listener.local_addr().context(LocalAddrSnafu { addr })?;

        let (mut stream, mut peer_addr) =
            listener.accept().await.context(AcceptingSnafu { addr })?;
        while !filter(peer_addr) {
            (stream, peer_addr) = listener.accept().await.context(AcceptingSnafu { addr })?;
        }

        (stream, local_addr, peer_addr)
    } else {
        let mut i = 0;

        let mut res = socket.connect(addr).await.context(ConnectingSnafu { addr });

        let stream = loop {
            match res {
                Ok(stream) => break stream,
                Err(e) => {
                    if let Some(max_retries) = max_retries {
                        if i >= max_retries {
                            return Err(e);
                        }

                        tokio::time::sleep(retry_sleep_duration).await;

                        let socket = get_socket(&addr, &tcp_settings)?;
                        res = socket.connect(addr).await.context(ConnectingSnafu { addr });

                        i += 1;
                    } else {
                        return Err(e);
                    }
                }
            }
        };

        let local_addr = stream.local_addr().context(LocalAddrSnafu { addr })?;
        let peer_addr = stream.peer_addr().expect("Should be able to get peer_addr");
        (stream, local_addr, peer_addr)
    };

    if let Some(nodelay) = tcp_settings.nodelay {
        stream
            .set_nodelay(nodelay)
            .context(SetNodelaySnafu { addr })?;
    }

    if let Some(ttl) = tcp_settings.ttl {
        stream.set_ttl(ttl).context(SetTtlSnafu { addr })?;
    }

    Ok(Channel {
        framed: Framed::new(stream, VariedLengthDelimitedCodec::new()),
        local_addr,
        peer_addr,
        _phantom: PhantomData,
    })
}

fn get_socket(addr: &SocketAddr, tcp_settings: &TcpSettings) -> Result<TcpSocket> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4().context(NewSocketSnafu { addr: *addr })?
    } else {
        TcpSocket::new_v6().context(NewSocketSnafu { addr: *addr })?
    };

    if let Some(reuseaddr) = tcp_settings.reuseaddr {
        socket
            .set_reuseaddr(reuseaddr)
            .context(SetReuseAddrSnafu { addr: *addr })?;
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    if let Some(reuseport) = tcp_settings.reuseport {
        socket
            .set_reuseport(reuseport)
            .context(SetReusePortSnafu { addr: *addr })?;
    }

    if let Some(linger) = tcp_settings.linger {
        socket
            .set_linger(linger)
            .context(SetLingerSnafu { addr: *addr })?;
    }

    if let Some(size) = tcp_settings.recv_buffer_size {
        socket
            .set_recv_buffer_size(size)
            .context(SetRecvBufferSizeSnafu { addr: *addr })?;
    }

    if let Some(size) = tcp_settings.send_buffer_size {
        socket
            .set_send_buffer_size(size)
            .context(SetSendBufferSizeSnafu { addr: *addr })?;
    }

    Ok(socket)
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelBuilderError {
        #[snafu(display("[ChannelBuilderError] Encountered unexpected error on spawned task for converting to socket addr"))]
        SpawnJoin {
            /// source Error
            source: JoinError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to convert input to socket address"))]
        ToSocketAddrs {
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to create Tcp Socket for {addr}"))]
        NewSocketError {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set reuseaddr for {addr}"))]
        SetReuseAddr {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set reuseport for {addr}"))]
        SetReusePort {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set linger for {addr}"))]
        SetLinger {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set recv_buffer_size for {addr}"))]
        SetRecvBufferSize {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set send_buffer_size for {addr}"))]
        SetSendBufferSize {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set nodelay for {addr}"))]
        SetNodelay {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to set ttl for {addr}"))]
        SetTtl {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to connect to {addr}"))]
        Connecting {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        /// returned from invalid inner IO Error
        #[snafu(display("[ChannelBuilderError] Failed to bind on {addr}"))]
        Binding {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        /// returned from invalid inner IO Error
        #[snafu(display("[ChannelBuilderError] Failed to listen on {addr}"))]
        Listening {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        /// returned from invalid inner IO Error
        #[snafu(display(
            "[ChannelBuilderError] Encountered IO Error while accepting connections on {addr}"
        ))]
        Accepting {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelBuilderError] Failed to get local addr for listener on {addr}"))]
        LocalAddr {
            addr: SocketAddr,
            source: io::Error,
            backtrace: Backtrace,
        },
    }
}
