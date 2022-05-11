//! Contains [BuilderFuture] which builds [multi_channel::Channel](super::Channel) when `.await`ed.
//!
//! [BuilderFuture] is returned by [channel_on](super::channel_on) function without awaiting it.
//!
//! Before awaiting the future, you can chain other methods on it to configure the Channel.
//!
//! To see all available configurations, see [BuilderFuture].

use super::Channel;
use crate::util::{stream_pool::StreamPool, Accept, TcpStreamSettings};
use errors::*;
use futures::{ready, Future};
use pin_project::pin_project;
use snafu::{Backtrace, ResultExt};
use std::io;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use std::time::Duration;
use tokio::net::{TcpListener, TcpSocket};
use tokio::task::JoinError;

pub type Result<T, E = BuilderError> = std::result::Result<T, E>;

pub(crate) fn new_multi<A: 'static + Send + Clone + ToSocketAddrs, T, E>(
    local_addr: A,
) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E>>>> {
    let tcp_settings = TcpSettings::default();
    let limit = None;

    BuilderFuture {
        local_addr: local_addr.clone(),
        tcp_settings,
        limit,
        fut: build_channel::<A, T, E, 0>(local_addr, limit, tcp_settings),
        _phantom: PhantomData,
    }
}

/// Future used to configure and build [Channel](super::Channel).
///
/// Use [channel_on](super::channel_on) function to create the [BuilderFuture].
///
/// You can chain any number of configurations to the future:
///
/// ```no_run
/// use tsyncp::multi_channel;
/// use serde::{Serialize, Deserialize};
/// use std::time::Duration;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy;
///
/// #[tokio::main]
/// async fn main() -> color_eyre::Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
///         .limit(20)              // limit the total number of possible connections to 20.
///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
///         .set_tcp_ttl(60_000)
///         .set_tcp_nodelay(true)
///         .set_tcp_reuseaddr(true)
///         .set_tcp_reuseport(true)
///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
///         .accept()               // accept 10 connections before returning.
///         .num(10)
///         .await?;
///
///     Ok(())
/// }
/// ```
///
/// However, there are some exclusive futures:
/// - You can only use one of [BuilderFuture::limit] and [BuilderFuture::limit_const].
#[derive(Debug)]
#[pin_project]
pub struct BuilderFuture<A, T, E, Fut, const N: usize = 0, L = TcpListener>
where
    Fut: Future<Output = Result<Channel<T, E, N, L>>>,
    L: Accept,
{
    local_addr: A,
    limit: Option<usize>,
    tcp_settings: TcpSettings,
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

impl<A, T, E, Fut, const N: usize, L> BuilderFuture<A, T, E, Fut, N, L>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    L: Accept,
    Fut: Future<Output = Result<Channel<T, E, N, L>>>,
{
    /// Before returning a [Channel], first accept the given number of connections.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .accept()               // accept 10 connections before returning.
    ///         .num(10)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn accept(
        self,
    ) -> AcceptBuilderFuture<
        Self,
        Channel<T, E, N, L>,
        T,
        E,
        N,
        L,
        impl FnMut(SocketAddr),
        impl FnMut(SocketAddr) -> bool,
    > {
        AcceptBuilderFuture::new(self, |_| {}, |_| true)
    }
}

impl<A, T, E, Fut, const N: usize> BuilderFuture<A, T, E, Fut, N>
where
    A: 'static + Send + Clone + ToSocketAddrs,
    Fut: Future<Output = Result<Channel<T, E, N>>>,
{
    /// Limit the total number of connections this channel can have.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .limit(10)                              // limit the total number of possible connections to 10.
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit(
        mut self,
        limit: usize,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.limit.replace(limit);

        self.refresh()
    }

    /// Limit the total number of connections this channel can have using const generic usize value.
    ///
    /// Use this method if you want to use an array instead of a vec for the [connection pool](crate::util::stream_pool::StreamPool)
    /// that handles all the connections.
    /// Using an array on the stack may improve performance by reducing access time to the streams.
    ///
    /// For more information about using an array or vec, see [StreamPool](crate::util::stream_pool::StreamPool).
    ///
    /// If you use this method, you must specify this value as the second paramter in the type
    /// specifier, as shown below.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy, 10> = multi_channel::channel_on("localhost:8000")
    ///         .limit_const::<10>()                //     ^--- this value must be set. Can be `_`.
    ///         .accept()
    ///         .to_limit()                         // accept up to the limit (10).
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn limit_const<const M: usize>(
        self,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, M>>>, M> {
        self.refresh()
    }

    /// Set tcp reuseaddr for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_reuseaddr(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_reuseaddr(
        mut self,
        reuseaddr: bool,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.reuseaddr.replace(reuseaddr);

        self.refresh()
    }

    /// Set tcp reuseport for all the connections made on this channel.
    ///
    /// *Warning:* only available to unix targets excluding "solaris" and "illumos".
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_reuseport(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    #[cfg_attr(
        docsrs,
        doc(cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))
    )]
    pub fn set_tcp_reuseport(
        mut self,
        reuseport: bool,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.reuseport.replace(reuseport);

        self.refresh()
    }

    /// Set tcp linger for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    /// use std::time::Duration;
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_linger(Some(Duration::from_millis(10_000)))
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_linger(
        mut self,
        dur: Option<Duration>,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.linger.replace(dur);

        self.refresh()
    }

    /// Set tcp nodelay for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_nodelay(true)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_nodelay(
        mut self,
        nodelay: bool,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.stream_settings.nodelay.replace(nodelay);

        self.refresh()
    }

    /// Set tcp ttl for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_ttl(60_000)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_ttl(
        mut self,
        ttl: u32,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.stream_settings.ttl.replace(ttl);

        self.refresh()
    }

    /// Set tcp recv_buffer_size for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_recv_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_recv_buffer_size(
        mut self,
        size: u32,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.recv_buffer_size.replace(size);

        self.refresh()
    }

    /// Set tcp send_buffer_size for all the connections made on this channel.
    ///
    /// ### Example:
    ///
    /// ```no_run
    /// use tsyncp::multi_channel;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy;
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:8000")
    ///         .set_tcp_send_buffer_size(8 * 1024 * 1024)
    ///         .await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn set_tcp_send_buffer_size(
        mut self,
        size: u32,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, N>>>, N> {
        self.tcp_settings.send_buffer_size.replace(size);

        self.refresh()
    }

    fn refresh<const M: usize>(
        self,
    ) -> BuilderFuture<A, T, E, impl Future<Output = Result<Channel<T, E, M>>>, M> {
        let BuilderFuture {
            local_addr,
            tcp_settings,
            limit,
            ..
        } = self;

        BuilderFuture {
            local_addr: local_addr.clone(),
            tcp_settings,
            limit,
            fut: build_channel::<A, T, E, M>(local_addr, limit, tcp_settings),
            _phantom: PhantomData,
        }
    }
}

impl<A, T, E, Fut, const N: usize, L> Future for BuilderFuture<A, T, E, Fut, N, L>
where
    Fut: Future<Output = Result<Channel<T, E, N, L>>>,
    L: Accept,
{
    type Output = Result<Channel<T, E, N, L>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx)
    }
}

#[derive(Debug)]
#[pin_project]
pub struct AcceptBuilderFuture<Fut, C, T, E, const N: usize, L, H, F> {
    #[pin]
    fut: Fut,
    num: usize,
    to_limit: bool,
    handle: H,
    filter: F,
    channel: Option<C>,
    _phantom: PhantomData<(T, E, L)>,
}

impl<T, C, E, Fut, const N: usize, L, H, F> AcceptBuilderFuture<Fut, C, T, E, N, L, H, F> {
    pub(crate) fn new(fut: Fut, handle: H, filter: F) -> Self {
        Self {
            fut,
            num: 1,
            to_limit: false,
            handle,
            filter,
            channel: None,
            _phantom: PhantomData,
        }
    }

    pub fn num(mut self, num: usize) -> Self {
        self.num = num;
        self.to_limit = false;

        self
    }

    pub fn to_limit(mut self) -> Self {
        self.to_limit = true;

        self
    }

    pub fn handle<H2>(self, handle: H2) -> AcceptBuilderFuture<Fut, C, T, E, N, L, H2, F>
    where
        H2: FnMut(SocketAddr),
    {
        let Self {
            fut,
            num,
            to_limit,
            filter,
            channel,
            ..
        } = self;

        AcceptBuilderFuture {
            fut,
            num,
            to_limit,
            handle,
            filter,
            channel,
            _phantom: PhantomData,
        }
    }

    pub fn filter<F2>(self, filter: F2) -> AcceptBuilderFuture<Fut, C, T, E, N, L, H, F2>
    where
        F2: FnMut(SocketAddr) -> bool,
    {
        let Self {
            fut,
            num,
            to_limit,
            handle,
            channel,
            ..
        } = self;

        AcceptBuilderFuture {
            fut,
            num,
            to_limit,
            handle,
            filter,
            channel,
            _phantom: PhantomData,
        }
    }
}

impl<T, C, E, Fut, const N: usize, L, H, F> Future for AcceptBuilderFuture<Fut, C, T, E, N, L, H, F>
where
    Fut: Future<Output = Result<C>>,
    C: AsMut<Channel<T, E, N, L>>,
    L: Accept,
    H: FnMut(SocketAddr),
    F: FnMut(SocketAddr) -> bool,
{
    type Output = Result<C, AcceptingError<L::Error>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Some(channel) = this.channel {
            let channel = channel.as_mut();
            let num = this
                .to_limit
                .then(|| channel.limit())
                .flatten()
                .unwrap_or(*this.num);

            while channel.len() < num {
                match ready!(channel.listener.poll_accept(&channel.stream_config, cx)) {
                    Ok((stream, addr)) => {
                        if (this.filter)(addr) {
                            channel
                                .stream_pool
                                .push_stream(stream, addr)
                                .expect("limit is checked above");

                            (this.handle)(addr);
                        }
                    }
                    Err(err) => return Poll::Ready(Err(err).context(AcceptingSnafu)),
                }
            }

            return Poll::Ready(Ok(this
                .channel
                .take()
                .expect("Channel is already returned")));
        }

        let channel = ready!(this.fut.poll(cx))?;

        this.channel.replace(channel);

        cx.waker().wake_by_ref();

        Poll::Pending
    }
}

async fn build_channel<A, T, E, const N: usize>(
    local_addr: A,
    limit: Option<usize>,
    tcp_settings: TcpSettings,
) -> Result<Channel<T, E, N, TcpListener>>
where
    A: 'static + Send + ToSocketAddrs,
{
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

    let pool: StreamPool<_, N> = if N > 0 {
        StreamPool::<_, N>::array()
    } else if let Some(limit) = limit {
        StreamPool::with_limit(limit)
    } else {
        StreamPool::vec()
    };

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
    pub enum BuilderError {
        #[snafu(display("[BuilderError] Encountered unexpected error on spawned task for converting to socket addr"))]
        SpawnJoin {
            /// source Error
            source: JoinError,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to convert input to socket address"))]
        ToSocketAddrs {
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] No addresses found"))]
        NoAddrFound { backtrace: Backtrace },
        #[snafu(display("[BuilderError] Failed to set reuseaddr for {addr}"))]
        SetReuseAddr {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set reuseport for {addr}"))]
        SetReusePort {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set linger for {addr}"))]
        SetLinger {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set recv_buffer_size for {addr}"))]
        SetRecvBufferSize {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set send_buffer_size for {addr}"))]
        SetSendBufferSize {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set nodelay for {addr}"))]
        SetNodelay {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to set ttl for {addr}"))]
        SetTtl {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to create Tcp Socket for {addr}"))]
        NewSocket {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        /// returned from invalid inner IO Error
        #[snafu(display("[BuilderError] Failed to bind on {addr}"))]
        Binding {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        /// returned from invalid inner IO Error
        #[snafu(display("[BuilderError] Failed to listen on {addr}"))]
        Listening {
            addr: SocketAddr,
            /// source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[BuilderError] Failed to get local addr for listener on {addr}"))]
        LocalAddr {
            addr: SocketAddr,
            source: io::Error,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum AcceptingError<LE>
    where
        LE: 'static + snafu::Error,
    {
        #[snafu(display(
            "[Builder Accepting Error] Failed to accept connection while building Channel"
        ))]
        Accepting { source: LE, backtrace: Backtrace },
        #[snafu(display("[Builder Error] Failed to build Channel"))]
        Building { source: BuilderError },
    }

    impl<LE> From<BuilderError> for AcceptingError<LE>
    where
        LE: 'static + snafu::Error,
    {
        fn from(e: BuilderError) -> Self {
            Self::Building { source: e }
        }
    }
}
