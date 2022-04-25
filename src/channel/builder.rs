use super::Channel;
use crate::util::{frame_codec::VariedLengthDelimitedCodec, split::TcpSplit, Framed};
use errors::*;
use futures::future::Ready;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use std::{fmt, io};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpSocket;
use tokio::task::JoinError;

pub type Result<T, E = ChannelBuilderError> = std::result::Result<T, E>;
pub type BuildResult<RW> = Result<(RW, SocketAddr, SocketAddr)>;

pub(crate) fn new<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
    listening: bool,
) -> ChannelBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    let max_retries = None;
    let retry_sleep_duration = Duration::from_millis(1000);
    let filter = |_| true;
    let tcp_settings = TcpSettings::default();

    ChannelBuilderFuture {
        dest: dest.clone(),
        listening,
        max_retries,
        retry_sleep_duration,
        tcp_settings,
        state: ChannelBuilderFutureState::Connecting(build_stream::<A, _>(
            dest,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            filter.clone(),
        )),
        filter,
        _phantom: PhantomData,
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Channel].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
#[pin_project]
pub struct ChannelBuilderFuture<A, T, E, RW, Fut, Filter> {
    dest: A,
    listening: bool,
    max_retries: Option<usize>,
    retry_sleep_duration: Duration,
    tcp_settings: TcpSettings,
    filter: Filter,
    #[pin]
    state: ChannelBuilderFutureState<RW, Fut>,
    _phantom: PhantomData<(T, E)>,
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

#[pin_project(project = BuilderStateEnum)]
enum ChannelBuilderFutureState<RW, Fut> {
    Connecting(#[pin] Fut),
    CustomStream(Option<(RW, SocketAddr, SocketAddr)>),
}

impl<A, T, E, RW, Fut, Filter> ChannelBuilderFuture<A, T, E, RW, Fut, Filter>
where
    A: 'static + Clone + Send + ToSocketAddrs,
    Filter: Clone + Fn(SocketAddr) -> bool,
{
    pub fn retry(
        mut self,
        retry_sleep_duration: Duration,
        max_retries: usize,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.max_retries.replace(max_retries);
        self.retry_sleep_duration = retry_sleep_duration;

        self.refresh()
    }

    pub fn filter<Filter2: Clone + Fn(SocketAddr) -> bool>(
        self,
        filter: Filter2,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter2>
    {
        let ChannelBuilderFuture {
            dest,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            ..
        } = self;

        ChannelBuilderFuture {
            dest: dest.clone(),
            listening,
            max_retries,
            retry_sleep_duration,
            state: ChannelBuilderFutureState::Connecting(build_stream::<A, Filter2>(
                dest,
                listening,
                max_retries,
                retry_sleep_duration,
                tcp_settings.clone(),
                filter.clone(),
            )),
            tcp_settings,
            filter,
            _phantom: PhantomData,
        }
    }

    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
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
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.reuseport.replace(reuseport);

        self.refresh()
    }

    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.linger.replace(dur);

        self.refresh()
    }

    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.nodelay.replace(nodelay);

        self.refresh()
    }

    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.ttl.replace(ttl);

        self.refresh()
    }

    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.recv_buffer_size.replace(size);

        self.refresh()
    }

    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        self.tcp_settings.send_buffer_size.replace(size);

        self.refresh()
    }

    fn refresh(
        self,
    ) -> ChannelBuilderFuture<A, T, E, TcpSplit, impl Future<Output = BuildResult<TcpSplit>>, Filter>
    {
        let ChannelBuilderFuture {
            dest,
            listening,
            max_retries,
            retry_sleep_duration,
            tcp_settings,
            filter,
            ..
        } = self;

        ChannelBuilderFuture {
            dest: dest.clone(),
            listening,
            max_retries,
            retry_sleep_duration,
            state: ChannelBuilderFutureState::Connecting(build_stream::<A, _>(
                dest,
                listening,
                max_retries,
                retry_sleep_duration,
                tcp_settings.clone(),
                filter.clone(),
            )),
            tcp_settings,
            filter,
            _phantom: PhantomData,
        }
    }
}

impl<A, T, E, RW, Fut, Filter> ChannelBuilderFuture<A, T, E, RW, Fut, Filter> {
    pub fn with_codec<C>(mut self) -> ChannelBuilderFuture<A, T, C, RW, Fut, Filter> {
        ChannelBuilderFuture {
            dest: self.dest,
            listening: self.listening,
            max_retries: self.max_retries,
            retry_sleep_duration: self.retry_sleep_duration,
            tcp_settings: self.tcp_settings,
            filter: self.filter,
            state: self.state,
            _phantom: PhantomData,
        }
    }

    pub fn with_stream<S>(
        mut self,
        custom_stream: S,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> ChannelBuilderFuture<A, T, E, S, Ready<Result<S>>, Filter> {
        ChannelBuilderFuture {
            dest: self.dest,
            listening: self.listening,
            max_retries: self.max_retries,
            retry_sleep_duration: self.retry_sleep_duration,
            tcp_settings: self.tcp_settings,
            filter: self.filter,
            state: ChannelBuilderFutureState::CustomStream(Some((
                custom_stream,
                local_addr,
                peer_addr,
            ))),
            _phantom: PhantomData,
        }
    }
}

impl<
        A,
        T,
        E,
        RW: AsyncRead + AsyncWrite + std::fmt::Debug,
        Fut: Future<Output = BuildResult<RW>>,
        Filter,
    > Future for ChannelBuilderFuture<A, T, E, RW, Fut, Filter>
where
    T: fmt::Debug,
{
    type Output = Result<Channel<T, E, RW>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        use BuilderStateEnum::*;
        let this = self.project();
        let (stream, local_addr, peer_addr) = match this.state.project() {
            Connecting(building) => {
                let build_result = ready!(building.poll(cx));

                match build_result {
                    Ok((stream, local_addr, peer_addr)) => (stream, local_addr, peer_addr),
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            CustomStream(stream_info) => stream_info.take().expect("CustomStream should be set"),
        };

        Poll::Ready(Ok::<_, ChannelBuilderError>(Channel {
            framed: Framed::new(stream, VariedLengthDelimitedCodec::new()),
            local_addr,
            peer_addr,
            _phantom: PhantomData,
        }))
    }
}

async fn build_stream<A: 'static + Send + ToSocketAddrs, Filter: Fn(SocketAddr) -> bool>(
    addr: A,
    listening: bool,
    max_retries: Option<usize>,
    retry_sleep_duration: Duration,
    tcp_settings: TcpSettings,
    filter: Filter,
) -> Result<(TcpSplit, SocketAddr, SocketAddr)> {
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

    let (r, w) = stream.into_split();

    Ok((TcpSplit::RW(r, w), local_addr, peer_addr))
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

    /// Codec's error type
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
