use crate::{
    channel, multi_channel, spsc,
    util::{
        codec::{DecodeMethod, EncodeMethod},
        split::{RWSplit, TcpSplit},
    },
};
use errors::*;
use futures::{ready, Future, Sink, SinkExt, Stream, StreamExt};
use snafu::{Backtrace, ResultExt};
use std::{
    fmt,
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};
use tokio::io::{AsyncRead, AsyncWrite};

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

pub fn broadcast_on<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    dest: A,
) -> builder::SenderBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = multi_channel::builder::AcceptResult>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new_sender(dest)
}

pub fn listen_to<A: 'static + Clone + Send + ToSocketAddrs, T, E>(
    local_addr: A,
) -> builder::ReceiverBuilderFuture<
    A,
    T,
    E,
    TcpSplit,
    impl Future<Output = channel::builder::BuildResult<TcpSplit>>,
    impl Clone + Fn(SocketAddr) -> bool,
> {
    builder::new_receiver(local_addr)
}

#[pin_project::pin_project]
pub struct Receiver<T: fmt::Debug, E, RW = TcpSplit>(#[pin] pub(crate) channel::Channel<T, E, RW>);

impl<T: fmt::Debug, E, RW> From<spsc::Receiver<T, E, RW>> for Receiver<T, E, RW> {
    fn from(sender: spsc::Receiver<T, E, RW>) -> Self {
        Receiver(sender.0)
    }
}

impl<T, E, RW> Receiver<T, E, RW>
where
    T: fmt::Debug,
{
    pub fn local_addr(&self) -> &SocketAddr {
        &self.0.local_addr()
    }

    pub fn peer_addr(&self) -> &SocketAddr {
        &self.0.peer_addr()
    }
}

impl<T, E, R, W> Receiver<T, E, RWSplit<R, W>>
where
    T: fmt::Debug,
{
    pub fn split(
        self,
    ) -> Result<(Self, spsc::Sender<T, E, RWSplit<R, W>>), channel::errors::SplitError> {
        let (r, w) = self.0.split()?;

        Ok((r.into(), w))
    }
}

impl<T: 'static + fmt::Debug + Clone, E: 'static + DecodeMethod<T>, RW: AsyncRead + Unpin>
    Receiver<T, E, RW>
{
    pub async fn recv(&mut self) -> Option<Result<T, ReceiverError<T, E>>> {
        self.next().await
    }
}

impl<T: 'static + fmt::Debug, E: 'static + DecodeMethod<T>, RW: AsyncRead> Stream
    for Receiver<T, E, RW>
{
    type Item = Result<T, ReceiverError<T, E>>;

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

#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sender<T, E, const N: usize = 0, RW = TcpSplit>(
    #[pin] multi_channel::Channel<T, E, N, RW>,
);

impl<T, E, const N: usize, RW> Sender<T, E, N, RW> {
    pub(crate) fn from_channel(channel: multi_channel::Channel<T, E, N, RW>) -> Self {
        Self(channel)
    }

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

impl<T, E, const N: usize, R, W> Sender<T, E, N, RWSplit<R, W>> {
    pub fn split(
        self,
    ) -> Result<
        (Self, crate::mpsc::Receiver<T, E, N, RWSplit<R, W>>),
        multi_channel::errors::SplitError,
    > {
        let readhalf_is_listener = false;
        let (receiver, sender) = self.0.split(readhalf_is_listener)?;

        Ok((sender, receiver))
    }
}

impl<T, E, const N: usize> Sender<T, E, N> {
    pub async fn accept(&mut self) -> Result<SocketAddr, SenderAcceptingError<TcpSplit>> {
        self.0.accept().await.context(SenderAcceptingSnafu)
    }
}

impl<
        T: 'static + fmt::Debug + Clone,
        E: 'static + EncodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncWrite + Unpin,
    > Sender<T, E, N, RW>
{
    pub async fn send(&mut self, item: T) -> Result<(), SenderError<T, E>> {
        SinkExt::send(self, item).await
    }

    pub async fn broadcast(&mut self, item: T) -> Result<(), SenderError<T, E>> {
        self.send(item).await
    }

    pub async fn send_to(
        &mut self,
        item: T,
        addrs: &[SocketAddr],
    ) -> Result<(), SenderError<T, E>> {
        self.0.send_to(item, addrs).await.context(SenderSnafu)
    }

    pub async fn send_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        item: T,
        filter: Filter,
    ) -> Result<(), SenderError<T, E>> {
        self.0
            .send_filtered(item, filter)
            .await
            .context(SenderSnafu)
    }
}

impl<
        T: 'static + fmt::Debug + Clone,
        E: 'static + EncodeMethod<T>,
        const N: usize,
        RW: 'static + fmt::Debug + AsyncWrite + Unpin,
    > Sink<T> for Sender<T, E, N, RW>
{
    type Error = SenderError<T, E>;

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
    #[snafu(display("[SenderAcceptingError] Failed to accept stream"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderAcceptingError<T: 'static + fmt::Debug> {
        source: multi_channel::errors::AcceptingError<T>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[SenderError] Failed to send item on broadcast::Receiver"))]
    #[snafu(visibility(pub(super)))]
    pub struct SenderError<T, E>
    where
        T: 'static + fmt::Debug,
        E: 'static + EncodeMethod<T>,
        E::Error: 'static + fmt::Debug + std::error::Error,
    {
        source: multi_channel::errors::ChannelSinkError<T, E>,
        backtrace: Backtrace,
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[ReceiverError] Failed to receiver item on broadcast::Sender"))]
    #[snafu(visibility(pub(super)))]
    pub struct ReceiverError<T, E>
    where
        T: 'static + fmt::Debug,
        E: 'static + DecodeMethod<T>,
        E::Error: 'static + fmt::Debug + std::error::Error,
    {
        source: channel::errors::ChannelStreamError<T, E>,
        backtrace: Backtrace,
    }
}
