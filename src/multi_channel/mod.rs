use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::stream_pool::StreamPool;
use crate::util::{
    accept::Accept,
    listener::{ListenerWrapper, ReadListener, WriteListener},
    split::Split,
};
use crate::{broadcast, mpsc};
use errors::*;
use futures::{ready, Future};
use futures::{Sink, SinkExt, Stream};
use snafu::{ensure, ResultExt};
use std::fmt;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

pub mod accept;
pub mod builder;
pub mod recv;
pub mod send;

#[cfg(feature = "json")]
pub type JsonChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::JsonCodec, N>;

#[cfg(feature = "protobuf")]
pub type ProtobufChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::ProtobufCodec, N>;

#[cfg(feature = "bincode")]
pub type BincodeChannel<T, const N: usize = 0> = Channel<T, crate::util::codec::BincodeCodec, N>;

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
    type Error = UnsplitError<<L::Output as Split>::Error>;

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
            .context(ListenerUnsplitSnafu)?
            .into_inner();

        let stream_pool = StreamPool::<L::Output, N>::unsplit(l_stream_pool, r_stream_pool)
            .context(StreamPoolUnsplitSnafu)?;

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
    type Item = Result<T, StreamError<E::Error>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let (frame, addr) = match ready!(self.project().stream_pool.poll_next(cx)) {
            Some((Ok(frame), addr)) => (frame, addr),
            Some((Err(error), addr)) => {
                return Poll::Ready(Some(Err(error).context(StreamSnafu { addr })))
            }
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).context(ItemDecodeSnafu { addr });

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
    pub fn send(&mut self, item: T) -> send::SendFuture<'_, T, E, N, L> {
        send::SendFuture::new(self, item)
    }
}

impl<T, E, const N: usize, L> Sink<T> for Channel<T, E, N, L>
where
    T: Clone,
    L: Accept,
    E: EncodeMethod<T>,
    L::Output: AsyncWrite + Unpin,
{
    type Error = SinkError<E::Error>;

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu {
            addr: *self.local_addr(),
        })?;

        self.stream_pool
            .start_send_unpin(encoded)
            .context(SinkSnafu {
                addr: *self.local_addr(),
            })?;

        Ok(())
    }

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_ready_unpin(cx)).context(SinkSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_flush_unpin(cx)).context(SinkSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.stream_pool.poll_close_unpin(cx)).context(SinkSnafu {
            addr: *self.local_addr(),
        });

        Poll::Ready(res)
    }
}

pub mod errors {
    use crate::util::{listener, stream_pool};
    use snafu::{Backtrace, Snafu};
    use std::io::ErrorKind;
    use std::net::SocketAddr;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[Encode Error] Failed to encode item on {addr}"))]
        ItemEncode {
            addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[Sink Error] Failed to send item on {addr}"))]
        SinkError {
            addr: SocketAddr,
            source: stream_pool::errors::SinkError,
            backtrace: Backtrace,
        },
    }

    impl<E> SinkError<E>
    where
        E: 'static + snafu::Error,
    {
        pub fn addr(&self) -> &SocketAddr {
            match self {
                Self::SinkError { addr, .. } => addr,
                Self::ItemEncode { addr, .. } => addr,
            }
        }

        pub fn as_errors(&self) -> Option<impl Iterator<Item = &stream_pool::errors::PollError>> {
            match self {
                Self::SinkError { source, .. } => Some(source.as_errors()),
                _ => None,
            }
        }

        pub fn into_errors(self) -> Option<impl Iterator<Item = stream_pool::errors::PollError>> {
            match self {
                Self::SinkError { source, .. } => Some(source.into_errors()),
                _ => None,
            }
        }

        pub fn as_io_errors(&self) -> Option<impl Iterator<Item = &std::io::Error>> {
            match self {
                Self::SinkError { source, .. } => Some(source.as_io_errors()),
                _ => None,
            }
        }

        pub fn into_io_errors(self) -> Option<impl Iterator<Item = std::io::Error>> {
            match self {
                Self::SinkError { source, .. } => Some(source.into_io_errors()),
                _ => None,
            }
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[Decode Error] Failed to decode frame of item on {addr}"))]
        ItemDecode {
            addr: SocketAddr,
            source: E,
            backtrace: Backtrace,
        },
        #[snafu(display("[Stream Error] Failed receive item on {addr}"))]
        StreamError {
            addr: SocketAddr,
            source: stream_pool::errors::PollError,
            backtrace: Backtrace,
        },
    }

    impl<E> StreamError<E>
    where
        E: snafu::Error,
    {
        pub fn addr(&self) -> &SocketAddr {
            match self {
                Self::StreamError { addr, .. } => addr,
                Self::ItemDecode { addr, .. } => addr,
            }
        }

        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::StreamError { source, .. } => source.as_io(),
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

        pub fn is_broken_pipe(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == ErrorKind::BrokenPipe)
                .unwrap_or_default()
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum UnsplitError<E: 'static + snafu::Error> {
        #[snafu(display("[Unsplit Error] Underlying channels' local addrs are different: {l_local_addr:?} != {r_local_addr:?}"))]
        UnequalLocalAddr {
            l_local_addr: SocketAddr,
            r_local_addr: SocketAddr,
        },
        #[snafu(display("[Unsplit Error] Underlying channels' stream configs are different"))]
        UnequalStreamConfig,
        #[snafu(display("[Unsplit Error] Failed to split underlying listener"))]
        ListenerUnsplitError {
            source: listener::errors::UnsplitError,
            backtrace: Backtrace,
        },
        #[snafu(display("[Unsplit Error] Failed to split underlying stream pool"))]
        StreamPoolUnsplitError {
            source: stream_pool::errors::UnsplitError<E>,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum AcceptError<E: 'static + snafu::Error> {
        #[snafu(display("[Accept Error] Underlying stream pool failed to accept"))]
        StreamPoolAcceptError { source: E, backtrace: Backtrace },
        #[snafu(display("[Accept Error] Failed to push accepted stream to stream pool"))]
        PushStream {
            source: stream_pool::errors::StreamPoolError,
            backtrace: Backtrace,
        },
    }
}
