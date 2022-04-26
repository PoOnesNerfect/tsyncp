use super::Channel;
use crate::util::Accept;
use crate::util::{stream_pool::StreamPool, TcpStreamSettings};
use errors::*;
use futures::Future;
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::io;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpListener, TcpSocket};
use tokio::task::JoinError;

pub type Result<T, E = ChannelBuilderError> = std::result::Result<T, E>;

pub(crate) fn new_multi<A: 'static + Send + Clone + ToSocketAddrs, T, E>(
    local_addr: A,
) -> ChannelBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = Result<Channel<T, E>>>,
> {
    let filter = |_| true;
    let tcp_settings = TcpSettings::default();

    ChannelBuilderFuture {
        local_addr: local_addr.clone(),
        tcp_settings,
        limit: None,
        accept: 0,
        fut: build_tcp_channel::<A, T, E, 0, _>(local_addr, None, 0, tcp_settings, filter.clone()),
        filter,
        _phantom: PhantomData,
    }
}

#[pin_project]
pub struct ChannelBuilderFuture<A, T, E, Filter, Fut, const N: usize = 0, L = TcpListener>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = Result<Channel<T, E, N, L>>>,
    L: Accept,
{
    local_addr: A,
    limit: Option<usize>,
    accept: usize,
    tcp_settings: TcpSettings,
    filter: Filter,
    #[pin]
    fut: Fut,
    _phantom: PhantomData<(T, E)>,
}

#[derive(Debug, Default, Clone, Copy)]
struct TcpSettings {
    reuseaddr: Option<bool>,
    reuseport: Option<bool>,
    linger: Option<Option<Duration>>,
    recv_buffer_size: Option<u32>,
    send_buffer_size: Option<u32>,
    stream_settings: TcpStreamSettings,
}

impl<
        A: 'static + Send + Clone + ToSocketAddrs,
        T,
        E,
        Filter: Clone + Fn(SocketAddr) -> bool,
        Fut: Future<Output = Result<Channel<T, E, N>>>,
        const N: usize,
    > ChannelBuilderFuture<A, T, E, Filter, Fut, N>
{
    pub fn accept(
        mut self,
        accept: usize,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.accept = accept;

        self.refresh()
    }

    pub fn accept_full(
        mut self,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        if N > 0 {
            self.accept = N;
        } else if let Some(limit) = self.limit {
            self.accept = limit;
        }

        self.refresh()
    }

    pub fn accept_filtered<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        accept: usize,
        filter: Filter2,
    ) -> ChannelBuilderFuture<A, T, E, Filter2, impl Future<Output = Result<Channel<T, E, M>>>, M>
    {
        let ChannelBuilderFuture {
            local_addr,
            limit,
            tcp_settings,
            ..
        } = self;

        ChannelBuilderFuture {
            local_addr: local_addr.clone(),
            tcp_settings,
            limit,
            accept,
            fut: build_tcp_channel::<A, T, E, M, Filter2>(
                local_addr,
                limit,
                accept,
                tcp_settings,
                filter.clone(),
            ),
            filter,
            _phantom: PhantomData,
        }
    }

    pub fn accept_filtered_full<const M: usize, Filter2: Clone + Fn(SocketAddr) -> bool>(
        mut self,
        filter: Filter2,
    ) -> ChannelBuilderFuture<A, T, E, Filter2, impl Future<Output = Result<Channel<T, E, M>>>, M>
    {
        if M > 0 {
            self.accept = N;
        } else if let Some(limit) = self.limit {
            self.accept = limit;
        }

        let accept = self.accept;
        self.accept_filtered(accept, filter)
    }

    pub fn limit(
        mut self,
        limit: usize,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.limit.replace(limit);

        self.refresh()
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, M>>>, M>
    {
        self.refresh()
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
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
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.reuseport.replace(reuseport);

        self.refresh()
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.linger.replace(dur);

        self.refresh()
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.stream_settings.nodelay.replace(nodelay);

        self.refresh()
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.stream_settings.ttl.replace(ttl);

        self.refresh()
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.recv_buffer_size.replace(size);

        self.refresh()
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, N>>>, N>
    {
        self.tcp_settings.send_buffer_size.replace(size);

        self.refresh()
    }

    fn refresh<const M: usize>(
        self,
    ) -> ChannelBuilderFuture<A, T, E, Filter, impl Future<Output = Result<Channel<T, E, M>>>, M>
    {
        let ChannelBuilderFuture {
            local_addr,
            tcp_settings,
            limit,
            accept,
            filter,
            ..
        } = self;

        ChannelBuilderFuture {
            local_addr: local_addr.clone(),
            tcp_settings,
            limit,
            accept,
            fut: build_tcp_channel::<A, T, E, M, Filter>(
                local_addr,
                limit,
                accept,
                tcp_settings,
                filter.clone(),
            ),
            filter,
            _phantom: PhantomData,
        }
    }
}

impl<A, T, E, Filter, Fut, const N: usize, L> Future
    for ChannelBuilderFuture<A, T, E, Filter, Fut, N, L>
where
    Filter: Fn(SocketAddr) -> bool,
    Fut: Future<Output = Result<Channel<T, E, N, L>>>,
    L: Accept,
{
    type Output = Result<Channel<T, E, N, L>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx)
    }
}

async fn build_tcp_channel<
    A: 'static + Send + ToSocketAddrs,
    T,
    E,
    const N: usize,
    Filter: Fn(SocketAddr) -> bool,
>(
    local_addr: A,
    limit: Option<usize>,
    accept: usize,
    tcp_settings: TcpSettings,
    filter: Filter,
) -> Result<Channel<T, E, N, TcpListener>> {
    let addr = tokio::task::spawn_blocking(move || {
        local_addr.to_socket_addrs()?.next().ok_or(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("Given input could not be parsed to socket addr"),
        ))
    })
    .await
    .context(SpawnJoinSnafu)?
    .context(ToSocketAddrsSnafu)?;

    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4().context(NewSocketSnafu { addr })?
    } else {
        TcpSocket::new_v6().context(NewSocketSnafu { addr })?
    };

    if let Some(reuseaddr) = tcp_settings.reuseaddr {
        socket
            .set_reuseaddr(reuseaddr)
            .context(SetReuseAddrSnafu { addr })?;
    }

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    if let Some(reuseport) = tcp_settings.reuseport {
        socket
            .set_reuseport(reuseport)
            .context(SetReusePortSnafu { addr })?;
    }

    if let Some(linger) = tcp_settings.linger {
        socket.set_linger(linger).context(SetLingerSnafu { addr })?;
    }

    if let Some(size) = tcp_settings.recv_buffer_size {
        socket
            .set_recv_buffer_size(size)
            .context(SetRecvBufferSizeSnafu { addr })?;
    }

    if let Some(size) = tcp_settings.send_buffer_size {
        socket
            .set_send_buffer_size(size)
            .context(SetSendBufferSizeSnafu { addr })?;
    }

    socket.bind(addr).context(BindingSnafu { addr })?;

    let listener = socket.listen(1024).context(ListeningSnafu { addr })?;

    let local_addr = listener.local_addr().context(LocalAddrSnafu { addr })?;

    let mut pool: StreamPool<_, N> = if N > 0 {
        StreamPool::<_, N>::array()
    } else if let Some(limit) = limit {
        StreamPool::with_limit(limit)
    } else {
        StreamPool::with_capacity(accept)
    };

    let accept = limit.map(|l| l.min(accept)).unwrap_or(accept);

    let mut i = 0;
    while i < accept {
        let (stream, addr) = listener.accept().await.context(AcceptingSnafu { addr })?;
        if filter(addr) {
            if let Some(nodelay) = tcp_settings.stream_settings.nodelay {
                stream
                    .set_nodelay(nodelay)
                    .context(SetNodelaySnafu { addr })?;
            }

            if let Some(ttl) = tcp_settings.stream_settings.ttl {
                stream.set_ttl(ttl).context(SetTtlSnafu { addr })?;
            }

            pool.push_stream(stream, addr)
                .expect("Element should be pushed til len");

            i += 1;
        }
    }

    Ok(Channel {
        listener,
        local_addr,
        stream_pool: pool,
        stream_config: tcp_settings.stream_settings,
        _phantom: PhantomData,
    })
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelBuilderError {
        /// Invalid length in frame header was received while decoding frame.
        #[snafu(display("[ChannelBuilderError] Received invalid frame length {len} while decoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
        InvalidDecodingFrameLength {
            /// given invalid frame length
            len: usize,
            /// max frame length
            max_frame_length: usize,
        },
        /// Invalid length in frame header was received while encoding frame.
        #[snafu(display("[ChannelBuilderError] Received invalid frame length {len} while encoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
        InvalidEncodingFrameLength {
            /// given invalid frame length
            len: usize,
            /// max frame length
            max_frame_length: usize,
        },
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
        #[snafu(display("[ChannelBuilderError] No addresses found"))]
        NoAddrFound { backtrace: Backtrace },
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
        #[snafu(display("[ChannelBuilderError] Failed to create Tcp Socket for {addr}"))]
        NewSocketError {
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
