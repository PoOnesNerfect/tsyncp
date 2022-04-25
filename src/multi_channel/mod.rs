use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::split::{ListenerSplit, ListenerSplitError, RWSplit, TcpSplit};
use crate::util::stream_pool::{
    errors::{StreamPoolError, StreamPoolPollError, StreamPoolSinkError},
    StreamPool,
};
use crate::util::TcpStreamSettings;
use crate::util::TypeName;
use bytes::BytesMut;
use errors::*;
use futures::future::poll_fn;
use futures::{ready, Future};
use futures::{Sink, SinkExt};
use futures::{Stream, StreamExt};
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod builder;

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
    TcpSplit,
    impl Future<Output = builder::AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new_multi(local_addr)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T, E, const N: usize = 0, RW = TcpSplit> {
    listener: ListenerSplit,
    local_addr: SocketAddr,
    #[pin]
    stream_pool: StreamPool<RW, N>,
    tcp_stream_settings: TcpStreamSettings,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, const N: usize, RW> Channel<T, E, N, RW> {
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

    fn into_parts(
        self,
    ) -> (
        ListenerSplit,
        StreamPool<RW, N>,
        SocketAddr,
        TcpStreamSettings,
    ) {
        let Channel {
            listener,
            local_addr,
            stream_pool,
            tcp_stream_settings,
            ..
        } = self;
        (listener, stream_pool, local_addr, tcp_stream_settings)
    }

    fn from_parts(
        listener: ListenerSplit,
        stream_pool: StreamPool<RW, N>,
        local_addr: SocketAddr,
        tcp_stream_settings: TcpStreamSettings,
    ) -> Self {
        Self {
            listener,
            local_addr,
            stream_pool,
            tcp_stream_settings,
            _phantom: PhantomData,
        }
    }
}

impl<T, E, const N: usize, R, W> Channel<T, E, N, RWSplit<R, W>> {
    pub fn split(
        self,
        readhalf_is_listener: bool,
    ) -> Result<
        (
            crate::mpsc::Receiver<T, E, N, RWSplit<R, W>>,
            crate::broadcast::Sender<T, E, N, RWSplit<R, W>>,
        ),
        SplitError,
    > {
        let (listener, stream_pool, local_addr, tcp_stream_settings) = self.into_parts();
        if !listener.is_listener() {
            return AlreadySplitSnafu.fail();
        }

        let (listener_sender, listener_receiver) = listener
            .split(readhalf_is_listener)
            .expect("Variant already checked");
        let (r, w) = stream_pool.split().context(StreamPoolSnafu)?;

        let (receiver_channel, sender_channel) = if readhalf_is_listener {
            (
                Channel::from_parts(listener_sender, r, local_addr, tcp_stream_settings),
                Channel::from_parts(listener_receiver, w, local_addr, tcp_stream_settings),
            )
        } else {
            (
                Channel::from_parts(listener_receiver, r, local_addr, tcp_stream_settings),
                Channel::from_parts(listener_sender, w, local_addr, tcp_stream_settings),
            )
        };

        let receiver = crate::mpsc::Receiver::from_channel(receiver_channel);
        let sender = crate::broadcast::Sender::from_channel(sender_channel);

        Ok((receiver, sender))
    }
}

impl<T, E, const N: usize> Channel<T, E, N> {
    pub async fn accept(&mut self) -> Result<SocketAddr, AcceptingError<TcpSplit>> {
        let (stream, addr) = self
            .listener
            .accept(self.tcp_stream_settings)
            .await
            .context(AcceptingSnafu)?;

        self.stream_pool
            .push_stream(stream, addr)
            .context(PushStreamSnafu)?;

        Ok(addr)
    }
}

impl<
        T: fmt::Debug,
        E: DecodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncRead + Unpin,
    > Channel<T, E, N, RW>
{
    pub async fn recv(&mut self) -> Option<Result<T, ChannelStreamError<T, E>>> {
        self.next().await
    }

    pub async fn recv_with_addr(
        &mut self,
    ) -> Option<(Result<T, ChannelStreamError<T, E>>, SocketAddr)> {
        poll_fn(|cx| {
            let (frame, addr) = match ready!(self.stream_pool.poll_next_unpin(cx)) {
                Some((Ok(frame), addr)) => (frame, addr),
                Some((Err(error), addr)) => {
                    return Poll::Ready(Some((Err(error).context(PollNextSnafu { addr }), addr)))
                }
                None => return Poll::Ready(None),
            };

            let decoded = E::decode(frame).context(FrameDecodeSnafu { addr });

            Poll::Ready(Some((decoded, addr)))
        })
        .await
    }

    pub async fn recv_frame(&mut self) -> Option<Result<BytesMut, ChannelStreamError<T, E>>> {
        poll_fn(|cx| {
            let frame = match ready!(self.stream_pool.poll_next_unpin(cx)) {
                Some((Ok(frame), _)) => frame,
                Some((Err(error), addr)) => {
                    return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
                }
                None => return Poll::Ready(None),
            };

            Poll::Ready(Some(Ok(frame)))
        })
        .await
    }

    pub async fn recv_frame_with_addr(
        &mut self,
    ) -> Option<(Result<BytesMut, ChannelStreamError<T, E>>, SocketAddr)> {
        poll_fn(|cx| {
            let (frame, addr) = match ready!(self.stream_pool.poll_next_unpin(cx)) {
                Some((Ok(frame), addr)) => (frame, addr),
                Some((Err(error), addr)) => {
                    return Poll::Ready(Some((Err(error).context(PollNextSnafu { addr }), addr)))
                }
                None => return Poll::Ready(None),
            };

            Poll::Ready(Some((Ok(frame), addr)))
        })
        .await
    }
}

impl<
        T: fmt::Debug + Clone,
        E: EncodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncWrite + Unpin,
    > Channel<T, E, N, RW>
{
    pub async fn send(&mut self, item: T) -> Result<(), ChannelSinkError<T, E>> {
        SinkExt::send(self, item).await
    }

    pub async fn send_to(
        &mut self,
        item: T,
        addrs: &[SocketAddr],
    ) -> Result<(), ChannelSinkError<T, E>> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.stream_pool
            .send_to(encoded, addrs)
            .await
            .with_context(|_| SendToSnafu {
                item,
                addrs: addrs.to_vec(),
            })?;

        Ok(())
    }

    pub async fn send_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        item: T,
        filter: Filter,
    ) -> Result<(), ChannelSinkError<T, E>> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.stream_pool
            .send_filtered(encoded, filter)
            .await
            .with_context(|_| SendFilteredSnafu { item })?;

        Ok(())
    }
}

impl<
        T: fmt::Debug,
        E: DecodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncRead + Unpin,
    > Stream for Channel<T, E, N, RW>
{
    type Item = Result<T, ChannelStreamError<T, E>>;

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

impl<
        T: fmt::Debug + Clone,
        E: EncodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncWrite + Unpin,
    > Sink<T> for Channel<T, E, N, RW>
{
    type Error = ChannelSinkError<T, E>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

        self.project()
            .stream_pool
            .start_send(encoded)
            .context(StartSendSnafu { item })?;

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
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelSinkError<T: fmt::Debug, E: EncodeMethod<T>> {
        #[snafu(display("[ChannelSinkError] Failed to encode item of type {item}"))]
        ItemEncode {
            #[snafu(implicit)]
            item: TypeName<T>,
            source: E::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed send_to to {addrs:?} for item {item:?}"))]
        SendTo {
            item: T,
            addrs: Vec<SocketAddr>,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed send_filtered for item {item:?}"))]
        SendFiltered {
            item: T,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed start_send for item {item:?}"))]
        StartSend {
            item: T,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_ready"))]
        PollReady {
            #[snafu(implicit)]
            item: TypeName<T>,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_flush"))]
        PollFlush {
            #[snafu(implicit)]
            item: TypeName<T>,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_close"))]
        PollClose {
            #[snafu(implicit)]
            item: TypeName<T>,
            source: StreamPoolSinkError,
            backtrace: Backtrace,
        },
    }

    impl<T: fmt::Debug, E: EncodeMethod<T>> ChannelSinkError<T, E> {
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
    pub enum SplitError {
        #[snafu(display(
        "[multi_channel::SplitError] broadcast::Sender can only be split into mpsc::Receiver once"
    ))]
        AlreadySplit,
        #[snafu(display("[multi_channel::SplitError] Failed to split underlying stream pool"))]
        StreamPool {
            source: StreamPoolError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum AcceptingError<T: 'static + fmt::Debug> {
        #[snafu(display("[AcceptingError] Failed to accept stream"))]
        Accepting {
            source: ListenerSplitError<T>,
            backtrace: Backtrace,
        },
        #[snafu(display("[AcceptingError] Failed to push accepted stream to stream pool"))]
        PushStream {
            source: StreamPoolError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelStreamError<T: fmt::Debug, E: DecodeMethod<T>> {
        #[snafu(display(
            "[ChannelStreamError] Failed to decode frame of item type {item} on {addr}"
        ))]
        FrameDecode {
            addr: SocketAddr,
            #[snafu(implicit)]
            item: TypeName<T>,
            source: E::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelStreamError] Failed poll_next on {addr}"))]
        PollNext {
            addr: SocketAddr,
            source: StreamPoolError,
            backtrace: Backtrace,
        },
    }

    impl<T: fmt::Debug, E: DecodeMethod<T>> ChannelStreamError<T, E> {
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
