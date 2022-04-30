use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::{split::Split, Framed};
use crate::{broadcast, mpsc};
use errors::*;
use futures::{ready, Future, Sink, SinkExt, Stream, StreamExt};
use snafu::{ensure, Backtrace, ResultExt};
use std::marker::PhantomData;
use std::net::{SocketAddr, ToSocketAddrs};
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

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
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = builder::Result<Channel<T, E>>>,
> {
    builder::new(dest, false)
}

pub fn channel_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ChannelBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = builder::Result<Channel<T, E>>>,
> {
    builder::new(local_addr, true)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Channel<T, E, S = TcpStream> {
    #[pin]
    framed: Framed<S>,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    _phantom: PhantomData<(T, E)>,
}

impl<T, E, S> Channel<T, E, S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.peer_addr
    }
}

impl<T, E, S> Channel<T, E, S>
where
    S: Split,
{
    pub fn split(
        self,
    ) -> (
        broadcast::Receiver<T, E, S::Left>,
        mpsc::Sender<T, E, S::Right>,
    ) {
        Split::split(self)
    }
}

impl<T, E, S> Split for Channel<T, E, S>
where
    S: Split,
{
    type Left = broadcast::Receiver<T, E, S::Left>;
    type Right = mpsc::Sender<T, E, S::Right>;
    type Error = ChannelUnsplitError<<S as Split>::Error>;

    fn split(self) -> (Self::Left, Self::Right) {
        let Channel {
            framed,
            local_addr,
            peer_addr,
            ..
        } = self;

        let (r, w) = framed.split();

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

        (r.into(), w.into())
    }

    fn unsplit(left: Self::Left, right: Self::Right) -> Result<Self, Self::Error> {
        let Channel {
            framed: l_framed,
            local_addr: l_local_addr,
            peer_addr: l_peer_addr,
            ..
        } = left.into();

        let Channel {
            framed: r_framed,
            local_addr: r_local_addr,
            peer_addr: r_peer_addr,
            ..
        } = right.into();

        ensure!(
            l_local_addr == r_local_addr,
            UnequalLocalAddrSnafu {
                l_local_addr,
                r_local_addr
            }
        );

        ensure!(
            l_peer_addr == r_peer_addr,
            UnequalPeerAddrSnafu {
                l_peer_addr,
                r_peer_addr
            }
        );

        let framed = <_>::unsplit(l_framed, r_framed).context(FramedUnsplitSnafu)?;

        Ok(Channel {
            framed,
            local_addr: l_local_addr,
            peer_addr: l_peer_addr,
            _phantom: PhantomData,
        })
    }
}

impl<T, E, S> Channel<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), ChannelSinkError<E::Error>> {
        SinkExt::send(self, item).await
    }
}

impl<T, E: EncodeMethod<T>, S> Sink<T> for Channel<T, E, S>
where
    S: AsyncWrite + Unpin,
{
    type Error = ChannelSinkError<E::Error>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let encoded = E::encode(&item).context(ItemEncodeSnafu)?;

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

impl<T: Clone, E: DecodeMethod<T>, S> Channel<T, E, S>
where
    S: AsyncRead + Unpin,
{
    pub async fn recv(&mut self) -> Option<Result<T, ChannelStreamError<E::Error>>> {
        StreamExt::next(self).await
    }
}

impl<T, E: DecodeMethod<T>, S> Stream for Channel<T, E, S>
where
    S: AsyncRead + Unpin,
{
    type Item = Result<T, ChannelStreamError<E::Error>>;

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
    use crate::util::frame_codec::errors::LengthDelimitedCodecError;

    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelSinkError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[ChannelSinkError] Failed to encode item"))]
        ItemEncode { source: E, backtrace: Backtrace },
        #[snafu(display("[ChannelSinkError] Failed start_send"))]
        StartSend {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_ready"))]
        PollReady {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_flush"))]
        PollFlush {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[ChannelSinkError] Failed poll_close"))]
        PollClose {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelStreamError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[ChannelStreamError] Failed to decode frame of data"))]
        FrameDecode { source: E, backtrace: Backtrace },
        #[snafu(display("[ChannelStreamError] Failed poll_read"))]
        PollRead {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum ChannelUnsplitError<E: 'static + snafu::Error> {
        #[snafu(display("[ChannelUnsplitError] Underlying channels' local addrs are different: {l_local_addr:?} != {r_local_addr:?}"))]
        UnequalLocalAddr {
            l_local_addr: SocketAddr,
            r_local_addr: SocketAddr,
        },
        #[snafu(display("[ChannelUnsplitError] Underlying channels' peer addrs are different: {l_peer_addr:?} != {r_peer_addr:?}"))]
        UnequalPeerAddr {
            l_peer_addr: SocketAddr,
            r_peer_addr: SocketAddr,
        },
        #[snafu(display("[ChannelUnsplitError] Failed to unsplit framed"))]
        FramedUnsplit { source: E, backtrace: Backtrace },
    }
}
