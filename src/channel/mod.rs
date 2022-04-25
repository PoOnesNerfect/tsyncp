use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::frame_codec::VariedLengthDelimitedCodec;
use crate::util::split::{split_framed, RWSplit};
use crate::util::{split::TcpSplit, Framed, TypeName};
use bytes::Bytes;
use errors::*;
use futures::{ready, Future, Sink, SinkExt, Stream, StreamExt};
use snafu::{Backtrace, ResultExt};
use std::fmt;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder, Encoder};

pub mod builder;

#[cfg(feature = "json")]
pub type JsonChannel<T> = Channel<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufChannel<T> = Channel<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvChannel<T> = Channel<T, crate::util::codec::RkyvCodec>;

pub fn channel_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ChannelBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new(dest, false)
}

pub fn channel_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ChannelBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new(local_addr, true)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T: fmt::Debug, E, RW = TcpSplit> {
    #[pin]
    framed: Framed<RW>,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, RW> Channel<T, E, RW>
where
    T: fmt::Debug,
{
    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.peer_addr
    }
}

impl<T, E, R, W> Channel<T, E, RWSplit<R, W>>
where
    T: fmt::Debug,
{
    pub fn split(
        self,
    ) -> Result<
        (
            crate::spsc::Receiver<T, E, RWSplit<R, W>>,
            crate::spsc::Sender<T, E, RWSplit<R, W>>,
        ),
        SplitError,
    > {
        let Channel {
            framed,
            local_addr,
            peer_addr,
            ..
        } = self;

        let (r, w) = split_framed(framed).context(FramedSplitSnafu)?;

        let r = Channel {
            framed: r,
            local_addr,
            peer_addr,
            _phantom: PhantomData,
        };

        let w = Channel {
            framed: w,
            local_addr,
            peer_addr,
            _phantom: PhantomData,
        };

        Ok((r.into(), w.into()))
    }
}

impl<T, E, RW> Channel<T, E, RW>
where
    T: 'static + fmt::Debug,
    E: 'static + EncodeMethod<T>,
    RW: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), ChannelSinkError<T, E>> {
        SinkExt::send(self, item).await
    }
}

impl<T: fmt::Debug, E: EncodeMethod<T>, RW: AsyncWrite> Sink<T> for Channel<T, E, RW> {
    type Error = ChannelSinkError<T, E>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu { item })?;

        self.project()
            .framed
            .start_send(encoded)
            .context(StartSendSnafu)?;

        Ok(())
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().framed.poll_ready(cx)).context(StartSendSnafu);

        Poll::Ready(res)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().framed.poll_flush(cx)).context(StartSendSnafu);

        Poll::Ready(res)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let res = ready!(self.project().framed.poll_close(cx)).context(StartSendSnafu);

        Poll::Ready(res)
    }
}

impl<T: 'static + fmt::Debug + Clone, E: 'static + DecodeMethod<T>, RW: AsyncRead + Unpin>
    Channel<T, E, RW>
{
    pub async fn recv(&mut self) -> Option<Result<T, ChannelStreamError<T, E>>> {
        StreamExt::next(self).await
    }
}

impl<T: fmt::Debug, E: DecodeMethod<T>, RW: AsyncRead> Stream for Channel<T, E, RW> {
    type Item = Result<T, ChannelStreamError<T, E>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let frame = match ready!(self.project().framed.poll_next(cx)) {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => return Poll::Ready(Some(Err(error).context(PollReadSnafu))),
            None => return Poll::Ready(None),
        };

        let decoded = E::decode(frame).context(FrameDecodeSnafu);

        Poll::Ready(Some(decoded))
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelSinkError<T: fmt::Debug, E: EncodeMethod<T>>
    where
        E::Error: 'static + std::fmt::Debug + std::error::Error,
    {
        #[snafu(display("Failed to encode item of {item:?} with {codec}"))]
        ItemEncode {
            item: T,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: E::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("Failed start_send for type {item} with {codec}"))]
        StartSend {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: <VariedLengthDelimitedCodec as Encoder<Bytes>>::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("Failed poll_ready for type {item} with {codec}"))]
        PollReady {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: <VariedLengthDelimitedCodec as Encoder<Bytes>>::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("Failed poll_flush for type {item} with {codec}"))]
        PollFlush {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: <VariedLengthDelimitedCodec as Encoder<Bytes>>::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("Failed poll_close for type {item} with {codec}"))]
        PollClose {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: <VariedLengthDelimitedCodec as Encoder<Bytes>>::Error,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelStreamError<T, E: DecodeMethod<T>> {
        #[snafu(display("Failed to decode frame of item type {item} with {codec}"))]
        FrameDecode {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: E::Error,
            backtrace: Backtrace,
        },
        #[snafu(display("Failed poll_read for item {item} with {codec}"))]
        PollRead {
            #[snafu(implicit)]
            item: TypeName<T>,
            #[snafu(implicit)]
            codec: TypeName<E>,
            source: <VariedLengthDelimitedCodec as Decoder>::Error,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum SplitError {
        #[snafu(display("[splitError] Channel can only be split into mpsc::Receiver once"))]
        AlreadySplit,
        #[snafu(display("[SplitError] Failed to split Framed"))]
        FramedSplit {
            source: crate::util::split::errors::SplitError,
            backtrace: Backtrace,
        },
    }
}
