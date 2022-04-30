use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::stream_pool::{
    errors::{StreamPoolError, StreamPoolPollError, StreamPoolSinkError},
    StreamPool,
};
use crate::util::{
    accept::Accept,
    listener::{ListenerWrapper, ReadListener, WriteListener},
    split::Split,
};
use crate::{broadcast, mpsc};
use bytes::BytesMut;
use errors::*;
use futures::future::poll_fn;
use futures::{ready, Future};
use futures::{Sink, SinkExt};
use futures::{Stream, StreamExt};
use snafu::{ensure, Backtrace, ResultExt};
use std::fmt;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

pub mod accept;
pub mod builder;
pub mod recv;

#[cfg(feature = "json")]
pub type JsonChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::JsonCodec, N>;

#[cfg(feature = "protobuf")]
pub type ProtobufChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::ProtobufCodec, N>;

#[cfg(feature = "rkyv")]
pub type RkyvChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::RkyvCodec, N>;

pub fn channel_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ChannelBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = builder::Result<Channel<T, E>>>,
> {
    builder::new_multi(local_addr)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T, E, const N: usize = 0, L = TcpListener>
where
    L: Accept,
{
    listener: L,
    local_addr: SocketAddr,
    #[pin]
    stream_pool: StreamPool<L::Output, N>,
    stream_config: L::Config,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
{
    pub fn len(&self) -> usize {
        self.stream_pool.len()
    }

    pub fn limit(&self) -> Option<usize> {
        self.stream_pool.limit()
    }

    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.stream_pool.addrs()
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    pub fn split(
        self,
    ) -> (
        mpsc::Receiver<T, E, N, ReadListener<L>>,
        broadcast::Sender<T, E, N, WriteListener<L>>,
    ) {
        Split::split(self)
    }
}

impl<T, E, const N: usize, L> Split for Channel<T, E, N, L>
where
    L: Accept,
    L::Output: Split,
    <L::Output as Split>::Left: fmt::Debug,
    <L::Output as Split>::Right: fmt::Debug,
{
    type Left = mpsc::Receiver<T, E, N, ReadListener<L>>;
    type Right = broadcast::Sender<T, E, N, WriteListener<L>>;
    type Error = ChannelUnsplitError<<L::Output as Split>::Error>;

    fn split(self) -> (Self::Left, Self::Right) {
        let Channel {
            listener,
            local_addr,
            stream_pool,
            stream_config,
            ..
        } = self;

        let wrapper: ListenerWrapper<L> = listener.into();
        let (r_listener, w_listener) = wrapper.split();
        let (r_pool, w_pool) = stream_pool.split();

        let receiver = Channel {
            listener: r_listener,
            local_addr,
            stream_pool: r_pool,
            stream_config: stream_config.clone(),
            _phantom: PhantomData,
        };

        let sender = Channel {
            listener: w_listener,
            local_addr,
            stream_pool: w_pool,
            stream_config,
            _phantom: PhantomData,
        };

        (receiver.into(), sender.into())
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        let Channel {
            listener: l_listener,
            local_addr: l_local_addr,
            stream_pool: l_stream_pool,
            stream_config: l_stream_config,
            ..
        } = left.into();

        let Channel {
            listener: r_listener,
            local_addr: r_local_addr,
            stream_pool: r_stream_pool,
            stream_config: r_stream_config,
            ..
        } = right.into();

        ensure!(
            l_local_addr == r_local_addr,
            UnequalLocalAddrSnafu {
                l_local_addr,
                r_local_addr
            }
        );

        ensure!(l_stream_config == r_stream_config, UnequalStreamConfigSnafu);

        let listener = ListenerWrapper::<L>::unsplit(l_listener, r_listener)
            .context(ListenerSnafu)?
            .into_inner();

        let stream_pool = StreamPool::<L::Output, N>::unsplit(l_stream_pool, r_stream_pool)
            .context(StreamPoolSnafu)?;

        Ok(Self {
            listener,
            stream_pool,
            local_addr: l_local_addr,
            stream_config: l_stream_config,
            _phantom: PhantomData,
        })
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
{
    pub fn accept(&mut self) -> accept::AcceptFuture<'_, T, E, N, L> {
        accept::AcceptFuture::new(self)
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    pub fn recv(&mut self) -> recv::RecvFuture<'_, T, E, N, L> {
        recv::RecvFuture::new(self)
    }
}

impl<T, E: DecodeMethod<T>, const N: usize, L: Accept> Stream for Channel<T, E, N, L>
where
    L::Output: AsyncRead + Unpin,
{
    type Item = Result<T, ChannelStreamError<E::Error>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let (frame, addr) = match ready!(self.project().stream_pool.poll_next(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).context(FrameDecodeSnafu { addr });

        Poll::Ready(Some(decoded))
    }
}

impl<T, E, const N: usize, L> Channel<T, E, N, L>
where
    T: Clone,
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), ChannelSinkError<E::Error>> {
        SinkExt::send(self, item).await
    }

    pub async fn send_to(
        &mut self,
        item: T,
        addrs: &[SocketAddr],
    ) -> Result<(), ChannelSinkError<E::Error>> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.stream_pool
            .send_to(encoded, addrs)
            .await
            .with_context(|_| SendToSnafu {
                addrs: addrs.to_vec(),
            })?;

        Ok(())
    }

    pub async fn send_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        item: T,
        filter: Filter,
    ) -> Result<(), ChannelSinkError<E::Error>> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.stream_pool
            .send_filtered(encoded, filter)
            .await
            .context(SendFilteredSnafu)?;

        Ok(())
    }
}

impl<T: Clone, E: EncodeMethod<T>, const N: usize, L: Accept> Sink<T> for Channel<T, E, N, L>
where
    L::Output: AsyncWrite + Unpin,
{
    type Error = ChannelSinkError<E::Error>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.project()
            .stream_pool
            .start_send(encoded)
            .context(StartSendSnafu)?;

        Ok(())
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().stream_pool.poll_ready(cx)).context(PollReadySnafu);

        Poll::Ready(res)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().stream_pool.poll_flush(cx)).context(PollFlushSnafu);

        Poll::Ready(res)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().stream_pool.poll_close(cx)).context(PollCloseSnafu);

        Poll::Ready(res)
    }
}

pub mod errors {
    use super::*;
    use crate::util::{
        listener::errors::UnsplitListenerError, stream_pool::errors::StreamPoolSplitError,
    };
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelSinkError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[ChannelSinkError] Failed to encode item"))]
        ItemEncode { source: E, backtrace: Backtrace },
        #[snafu(display("[ChannelSinkError] Failed send_to"))]
        SendTo {
            addrs: Vec<SocketAddr>,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed send_filtered"))]
        SendFiltered {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed start_send"))]
        StartSend {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_ready"))]
        PollReady {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_flush"))]
        PollFlush {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_close"))]
        PollClose {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
    }

    impl<E> ChannelSinkError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::ItemEncode { .. } => None,
                Self::SendTo { source, .. } => source.as_io(),
                Self::SendFiltered { source, .. } => source.as_io(),
                Self::StartSend { source, .. } => source.as_io(),
                Self::PollReady { source, .. } => source.as_io(),
                Self::PollFlush { source, .. } => source.as_io(),
                Self::PollClose { source, .. } => source.as_io(),
            }
        }

        pub fn errors(&self) -> Option<impl Iterator<Item = &StreamPoolPollError>> {
            match self {
                Self::ItemEncode { .. } => None,
                Self::SendTo { source, .. } => Some(source.errors()),
                Self::SendFiltered { source, .. } => Some(source.errors()),
                Self::StartSend { source, .. } => Some(source.errors()),
                Self::PollReady { source, .. } => Some(source.errors()),
                Self::PollFlush { source, .. } => Some(source.errors()),
                Self::PollClose { source, .. } => Some(source.errors()),
            }
        }

        pub fn io_errors(&self) -> Option<impl Iterator<Item = &std::io::Error>> {
            match self {
                Self::ItemEncode { .. } => None,
                Self::SendTo { source, .. } => Some(source.io_errors()),
                Self::SendFiltered { source, .. } => Some(source.io_errors()),
                Self::StartSend { source, .. } => Some(source.io_errors()),
                Self::PollReady { source, .. } => Some(source.io_errors()),
                Self::PollFlush { source, .. } => Some(source.io_errors()),
                Self::PollClose { source, .. } => Some(source.io_errors()),
            }
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelUnsplitError<E: 'static + snafu::Error> {
        #[snafu(display("[ChannelUnsplitError] Underlying channels' local addrs are different: {l_local_addr:?} != {r_local_addr:?}"))]
        UnequalLocalAddr {
            l_local_addr: SocketAddr,
            r_local_addr: SocketAddr,
        },
        #[snafu(display(
            "[ChannelUnsplitError] Underlying channels' stream configs are different"
        ))]
        UnequalStreamConfig,
        #[snafu(display("[ChannelUnsplitError] Failed to split underlying listener"))]
        Listener {
            source: UnsplitListenerError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelUnsplitError] Failed to split underlying stream pool"))]
        StreamPool {
            source: StreamPoolSplitError<E>,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SinkAcceptError<LE, EE>
    where
        LE: 'static + snafu::Error,
        EE: 'static + snafu::Error,
    {
        #[snafu(display("[ChannelEncodeError] Failed to encode item"))]
        EncodeError { source: EE, backtrace: Backtrace },
        #[snafu(display("[ChannelAcceptError] Failed to accept stream"))]
        AcceptError {
            source: AcceptingError<LE>,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelStartSend] Failed to start sending item"))]
        AcceptAndStartSend {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelPollReadyError] failed to send item"))]
        AcceptAndPollReadyError {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelPollFlushError] failed to flush streams"))]
        AcceptAndPollFlushError {
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum AcceptingError<E: 'static + snafu::Error> {
        #[snafu(display("[AcceptingError] Failed to accept stream"))]
        Accepting { source: E, backtrace: Backtrace },
        #[snafu(display("[AcceptingError] Failed to push accepted stream to stream pool"))]
        PushStream {
            source: StreamPoolError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelStreamError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[ChannelStreamError] Failed to decode frame of item on {addr}"))]
        FrameDecode {
            addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelStreamError] Failed poll_next on {addr}"))]
        PollNext {
            addr: SocketAddr,
            source: StreamPoolError,
            backtrace: Backtrace,
        },
    }

    impl<E> ChannelStreamError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn addr(&self) -> &SocketAddr {
            match self {
                Self::FrameDecode { addr, .. } => addr,
                Self::PollNext { addr, .. } => addr,
            }
        }
    }

    impl<E> ChannelStreamError<E>
    where
        E: snafu::Error,
    {
        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::PollNext { source, .. } => source.as_io(),
                _ => None,
            }
        }

        pub fn is_connection_reset(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionReset)
                .unwrap_or_default()
        }

        pub fn is_connection_refused(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionRefused)
                .unwrap_or_default()
        }

        pub fn is_connection_aborted(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::ConnectionAborted)
                .unwrap_or_default()
        }

        pub fn is_not_connected(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::NotConnected)
                .unwrap_or_default()
        }
    }
}
