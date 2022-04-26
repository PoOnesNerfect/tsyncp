use crate::channel;
use crate::util::codec::{DecodeMethod, EncodeMethod};
use crate::util::tcp;
// use crate::{broadcast, channel, mpsc};
use errors::*;
use futures::Sink;
use futures::Stream;
use futures::{ready, StreamExt};
use futures::{Future, SinkExt};
use snafu::{Backtrace, ResultExt};
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod builder;

#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T> = Sender<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvSender<T> = Sender<T, crate::util::codec::RkyvCodec>;

#[cfg(feature = "json")]
pub type JsonReceiver<T> = Receiver<T, crate::util::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufReceiver<T> = Receiver<T, crate::util::codec::ProtobufCodec>;

#[cfg(feature = "rkyv")]
pub type RkyvReceiver<T, const N: usize = 0> = Receiver<T, crate::util::codec::RkyvCodec>;

pub fn send_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::SenderToBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::sender_to(dest)
}

pub fn send_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::SenderOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::sender_on(local_addr)
}

pub fn recv_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::ReceiverToBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::receiver_to(dest)
}

pub fn recv_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverOnBuilderFuture<
    A,
    T,
    E,
    impl Clone + Fn(SocketAddr) -> bool,
    impl Future<Output = channel::builder::Result<channel::Channel<T, E>>>,
> {
    builder::receiver_on(local_addr)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, S = tcp::OwnedWriteHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Sender<T, E, S> {
    fn from(channel: channel::Channel<T, E, S>) -> Self {
        Sender(channel)
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

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Receiver<T, E, S = tcp::OwnedReadHalf>(#[pin] channel::Channel<T, E, S>);

impl<T, E, S> From<channel::Channel<T, E, S>> for Receiver<T, E, S> {
    fn from(channel: channel::Channel<T, E, S>) -> Self {
        Receiver(channel)
    }
}

impl<T, E, S> Receiver<T, E, S> {
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, S> Receiver<T, E, S>
where
    E: DecodeMethod<T>,
    S: AsyncRead + Unpin,
{
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

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(display("[spsc::SenderError] Failed to send item"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderError<E>
    where
        E: 'static + std::error::Error,
    {
        source: channel::errors::ChannelSinkError<E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[spsc::ReceiverError] Failed to receiver item"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverError<E>
    where
        E: 'static + std::error::Error,
    {
        source: channel::errors::ChannelStreamError<E>,
        backtrace: Backtrace,
    }
}
