use super::Channel;
use crate::util::{split::TcpSplit, stream_pool::StreamPool, TcpStreamSettings};
use errors::*;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, OptionExt, ResultExt};
use std::io;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpListener, TcpSocket};
use tokio::task::JoinError;

type Result<T, E = ChannelBuilderError> = std::result::Result<T, E>;

pub type AcceptResult<const N: usize = 0, S = TcpSplit> =
    Result<(TcpListener, StreamPool<S, N>, SocketAddr)>;

pub(crate) fn new_multi<A: 'static + Send + Clone + ToSocketAddrs, T, E>(
    local_addr: A,
) -> ChannelBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    let filter = |_| true;
    let tcp_settings = TcpSettings::default();

    ChannelBuilderFuture {
        local_addr: local_addr.clone(),
        tcp_settings,
        limit: None,
        accept: 0,
        connecting: build_listener::<A, 0, _>(local_addr, None, 0, tcp_settings, filter.clone()),
        filter,
        _phantom: PhantomData,
    }
}

#[pin_project]
pub struct ChannelBuilderFuture<
    A,
    T,
    E,
    RW,
    Fut: Future<Output = AcceptResult<N, RW>>,
    Filter: Fn(SocketAddr) -> bool,
    const N: usize = 0,
> {
    local_addr: A,
    limit: Option<usize>,
    accept: usize,
    #[pin]
    connecting: Fut,
    tcp_settings: TcpSettings,
    filter: Filter,
    _phantom: PhantomData<(T, E, RW)>,
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
        A,
        T,
        E,
        RW,
        Fut: Future<Output = AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > ChannelBuilderFuture<A, T, E, RW, Fut, Filter, N>
{
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit.replace(limit);

        self
    }

    pub fn with_codec<C>(mut self) -> ChannelBuilderFuture<A, T, C, RW, Fut, Filter, N> {
        ChannelBuilderFuture {
            local_addr: self.local_addr,
            limit: self.limit,
            accept: self.accept,
            filter: self.filter,
            tcp_settings: self.tcp_settings,
            connecting: self.connecting,
            _phantom: PhantomData,
        }
    }
}

impl<
        A,
        T,
        E,
        Fut: Future<Output = AcceptResult<N>>,
        Filter: Clone + Fn(SocketAddr) -> bool,
        const N: usize,
    > ChannelBuilderFuture<A, T, E, TcpSplit, Fut, Filter, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
{
    pub fn accept(
        mut self,
        accept: usize,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.accept = accept;

        self.refresh()
    }

    pub fn accept_full(
        mut self,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
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
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<M>>, Filter2, M>
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
            connecting: build_listener::<A, M, Filter2>(
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
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<M>>, Filter2, M>
    {
        if M > 0 {
            self.accept = N;
        } else if let Some(limit) = self.limit {
            self.accept = limit;
        }

        let accept = self.accept;
        self.accept_filtered(accept, filter)
    }

    pub fn limit_const<const M: usize>(
        self,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<M>>, Filter, M>
    {
        self.refresh()
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
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
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.reuseport.replace(reuseport);

        self.refresh()
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.linger.replace(dur);

        self.refresh()
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.stream_settings.nodelay.replace(nodelay);

        self.refresh()
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.stream_settings.ttl.replace(ttl);

        self.refresh()
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.recv_buffer_size.replace(size);

        self.refresh()
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<N>>, Filter, N>
    {
        self.tcp_settings.send_buffer_size.replace(size);

        self.refresh()
    }

    fn refresh<const M: usize>(
        self,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = AcceptResult<M>>, Filter, M>
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
            connecting: build_listener::<A, M, Filter>(
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

impl<
        A,
        T,
        E,
        RW,
        Fut: Future<Output = AcceptResult<N, RW>>,
        Filter: Fn(SocketAddr) -> bool,
        const N: usize,
    > Future for ChannelBuilderFuture<A, T, E, RW, Fut, Filter, N>
{
    type Output = Result<Channel<T, E, N, RW>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let build_result = ready!(this.connecting.poll(cx));

        let (listener, stream_pool, local_addr) = match build_result {
            Ok(ret) => ret,
            Err(error) => return Poll::Ready(Err(error)),
        };

        Poll::Ready(Ok::<_, ChannelBuilderError>(Channel {
            listener: listener.into(),
            local_addr,
            stream_pool,
            tcp_stream_settings: this.tcp_settings.stream_settings,
            _phantom: PhantomData,
        }))
    }
}

async fn build_listener<
    A: 'static + Send + ToSocketAddrs,
    const N: usize,
    Filter: Fn(SocketAddr) -> bool,
>(
    local_addr: A,
    limit: Option<usize>,
    accept: usize,
    tcp_settings: TcpSettings,
    filter: Filter,
) -> AcceptResult<N> {
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

            let (r, w) = stream.into_split();
            pool.push_stream(TcpSplit::RW(r, w), addr)
                .expect("Element should be pushed til len");

            i += 1;
        }
    }

    Ok((listener, pool, local_addr))
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
