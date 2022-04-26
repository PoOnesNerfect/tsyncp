use crate::{
    channel, multi_channel,
    util::{
        codec::{DecodeMethod, EncodeMethod},
        tcp, Accept, ReadListener,
    },
};
use bytes::BytesMut;
use errors::*;
use futures::{ready, Future, Sink, SinkExt, Stream, StreamExt};
use snafu::{Backtrace, ResultExt};
use std::{
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

pub mod builder;

#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "json")]
pub type JsonReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::JsonCodec, N>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T> = Sender<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufReceiver<T, const N: usize = 0> =
    Receiver<T, crate::util::codec::ProtobufCodec, N>;

#[cfg(feature = "rkyv")]
pub type RkyvSender<T, const N: usize = 0> = Sender<T, crate::util::codec::RkyvCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::RkyvCodec, N>;

pub fn send_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::new_sender(dest)
}

pub fn recv_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    builder::new_receiver(local_addr)
}

#[pin_project::pin_project]
pub struct Sender<T, E, S = tcp::OwnedWriteHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Sender<T, E, S> {
    fn from(c: channel::Channel<T, E, S>) -> Self {
        Self(c)
    }
}

impl<T, E, S> Sender<T, E, S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, S> Sender<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), SenderError<E::Error>> {
        SinkExt::send(self, item).await
    }
}

impl<T, E, S> Sink<T> for Sender<T, E, S>
where
    E: EncodeMethod<T>,
    S: AsyncWrite + Unpin,
{
    type Error = SenderError<E::Error>;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.project().0.start_send(item).context(SenderSnafu)
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_ready(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_flush(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if let Err(e) = ready!(self.project().0.poll_close(cx)) {
            return Poll::Ready(Err(e).context(SenderSnafu));
        }

        Poll::Ready(Ok(()))
    }
}

#[pin_project::pin_project]
pub struct Receiver<T, E, const N: usize = 0, L: Accept = ReadListener<TcpListener>>(
    #[pin] multi_channel::Channel<T, E, N, L>,
);

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    L: Accept,
{
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn limit(&self) -> Option<usize> {
        self.0.limit()
    }

    pub fn local_addr(&self) -> &SocketAddr {
        self.0.local_addr()
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.0.peer_addrs()
    }
}

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    L: Accept,
    L::Error: 'static,
{
    pub async fn accept(&mut self) -> Result<SocketAddr, ReceiverAcceptingError<L::Error>> {
        self.0.accept().await.context(ReceiverAcceptingSnafu)
    }
}

impl<T, E, const N: usize, L> Receiver<T, E, N, L>
where
    E: DecodeMethod<T>,
    L: Accept,
    L::Output: AsyncRead + Unpin,
{
    pub async fn recv(&mut self) -> Option<Result<T, ReceiverError<E::Error>>> {
        self.0.next().await.map(|res| res.context(ReceiverSnafu))
    }

    pub async fn recv_with_addr(
        &mut self,
    ) -> Option<(Result<T, ReceiverError<E::Error>>, SocketAddr)> {
        self.0
            .recv_with_addr()
            .await
            .map(|(res, addr)| (res.context(ReceiverSnafu), addr))
    }

    pub async fn recv_frame(&mut self) -> Option<Result<BytesMut, ReceiverError<E::Error>>> {
        self.0
            .recv_frame()
            .await
            .map(|res| res.context(ReceiverSnafu))
    }

    pub async fn recv_frame_with_addr(
        &mut self,
    ) -> Option<(Result<BytesMut, ReceiverError<E::Error>>, SocketAddr)> {
        self.0
            .recv_frame_with_addr()
            .await
            .map(|(res, addr)| (res.context(ReceiverSnafu), addr))
    }
}

impl<T, E, const N: usize, L> Stream for Receiver<T, E, N, L>
where
    E: DecodeMethod<T>,
    L: Accept,
    L::Output: AsyncRead + Unpin,
{
    type Item = Result<T, ReceiverError<E::Error>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match ready!(self.project().0.poll_next(cx)) {
            Some(Ok(item)) => Poll::Ready(Some(Ok(item))),
            Some(Err(e)) => Poll::Ready(Some(Err(e).context(ReceiverSnafu))),
            None => Poll::Ready(None),
        }
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[mpsc::ReceiverAcceptingError] Failed to accept stream"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverAcceptingError<E: 'static + std::error::Error> {
        source: multi_channel::errors::AcceptingError<E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[mpsc::SenderError] Failed to send item"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderError<E>
    where
        E: 'static + std::error::Error,
    {
        source: channel::errors::ChannelSinkError<E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[mpsc::ReceiverError] Failed to receiver item on mpsc::Receiver"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverError<E>
    where
        E: 'static + std::error::Error,
    {
        source: multi_channel::errors::ChannelStreamError<E>,
        backtrace: Backtrace,
    }
}
