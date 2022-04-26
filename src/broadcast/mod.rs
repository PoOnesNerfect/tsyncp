use crate::{
    channel, multi_channel,
    util::{
        codec::{DecodeMethod, EncodeMethod},
        tcp, Accept, WriteListener,
    },
};
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
pub type JsonReceiver<T> = Receiver<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "json")]
pub type JsonSender<T, const N: usize = 0> = Sender<T, crate::util::codec::JsonCodec, N>;

#[cfg(feature = "protobuf")]
pub type ProtobufReceiver<T> = Receiver<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T, const N: usize = 0> = Sender<T, crate::util::codec::ProtobufCodec, N>;

#[cfg(feature = "rkyv")]
pub type RkyvReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::RkyvCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvSender<T, const N: usize = 0> = Sender<T, crate::util::codec::RkyvCodec, N>;

pub fn send_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = multi_channel::builder::Result<multi_channel::Channel<T, E>>>,
> {
    builder::new_sender(local_addr)
}

pub fn recv_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::new_receiver(dest)
}

#[pin_project::pin_project]
pub struct Receiver<T, E, S = tcp::OwnedReadHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> Receiver<T, E, S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T: Clone, E: DecodeMethod<T>, S: AsyncRead + Unpin> Receiver<T, E, S> {
    pub async fn recv(&mut self) -> Option<Result<T, ReceiverError<E::Error>>> {
        self.next().await
    }
}

impl<T, E, S> Stream for Receiver<T, E, S>
where
    E: DecodeMethod<T>,
    S: AsyncRead + Unpin,
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

#[pin_project::pin_project]
pub struct Sender<T, E, const N: usize = 0, L: Accept = WriteListener<TcpListener>>(
    #[pin] multi_channel::Channel<T, E, N, L>,
);

impl<T, E, const N: usize, L> Sender<T, E, N, L>
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

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    L: Accept,
{
    pub async fn accept(&mut self) -> Result<SocketAddr, SenderAcceptingError<L::Error>> {
        self.0.accept().await.context(SenderAcceptingSnafu)
    }
}

impl<T, E, const N: usize, L> Sender<T, E, N, L>
where
    T: Clone,
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
{
    pub async fn send(&mut self, item: T) -> Result<(), SenderError<E::Error>> {
        SinkExt::send(self, item).await
    }

    pub async fn broadcast(&mut self, item: T) -> Result<(), SenderError<E::Error>> {
        self.send(item).await
    }

    pub async fn send_to(
        &mut self,
        item: T,
        addrs: &[SocketAddr],
    ) -> Result<(), SenderError<E::Error>> {
        self.0.send_to(item, addrs).await.context(SenderSnafu)
    }

    pub async fn send_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        item: T,
        filter: Filter,
    ) -> Result<(), SenderError<E::Error>> {
        self.0
            .send_filtered(item, filter)
            .await
            .context(SenderSnafu)
    }
}

impl<T, E, const N: usize, L> Sink<T> for Sender<T, E, N, L>
where
    T: Clone,
    E: EncodeMethod<T>,
    L: Accept,
    L::Output: AsyncWrite + Unpin,
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

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[broadcast::SenderAcceptingError] Failed to accept stream"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderAcceptingError<E>
    where
        E: 'static + std::error::Error,
    {
        source: multi_channel::errors::AcceptingError<E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[broadcast::SenderError] Failed to send item"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderError<E>
    where
        E: 'static + std::error::Error,
    {
        source: multi_channel::errors::ChannelSinkError<E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[broadcast::ReceiverError] Failed to receiver item"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverError<E>
    where
        E: 'static + std::error::Error,
    {
        source: channel::errors::ChannelStreamError<E>,
        backtrace: Backtrace,
    }
}
