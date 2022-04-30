use super::{
    errors::{AcceptingError, ChannelSinkError, ChannelStreamError},
    AcceptingSnafu, Channel, FrameDecodeSnafu, ItemEncodeSnafu, PollCloseSnafu, PollFlushSnafu,
    PollNextSnafu, PollReadySnafu, PushStreamSnafu, SendFilteredSnafu, SendToSnafu, StartSendSnafu,
};
use crate::util::{
    accept::Accept,
    codec::{DecodeMethod, EncodeMethod},
    stream_pool::StreamPool,
};
use bytes::BytesMut;
use futures::{future::poll_fn, ready, Future, FutureExt, Sink, SinkExt, Stream, StreamExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{marker::PhantomData, net::SocketAddr, task::Poll};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug)]
#[pin_project]
pub struct ChannelRef<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    local_addr: SocketAddr,
    #[pin]
    stream_pool: &'pin mut StreamPool<L::Output, N>,
    _phantom: PhantomData<(T, E)>,
}

impl<'pin, T, E, const N: usize, L> ChannelRef<'pin, T, E, N, L>
where
    L: Accept,
{
    pub fn new(stream_pool: &'pin mut StreamPool<L::Output, N>, local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            stream_pool,
            _phantom: PhantomData,
        }
    }

    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.stream_pool.addrs()
    }

    pub fn len(&self) -> usize {
        self.stream_pool.len()
    }

    pub fn limit(&self) -> Option<usize> {
        self.stream_pool.limit()
    }
}

impl<'pin, T, E, const N: usize, L> ChannelRef<'pin, T, E, N, L>
where
    L: Accept,
    E: DecodeMethod<T>,
    L::Output: AsyncRead + Unpin,
{
    pub async fn recv(&mut self) -> Option<Result<T, ChannelStreamError<E::Error>>> {
        StreamExt::next(self).await
    }

    pub async fn recv_with_addr(
        &mut self,
    ) -> Option<Result<(T, SocketAddr), ChannelStreamError<E::Error>>> {
        poll_fn(|cx| {
            let (frame, addr) = match ready!(self.stream_pool.poll_next_unpin(cx)) {
                Some((Ok(frame), addr)) => (frame, addr),
                Some((Err(error), addr)) => {
                    return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
                }
                None => return Poll::Ready(None),
            };

            let decoded = E::decode(frame).with_context(|_| FrameDecodeSnafu { addr });

            Poll::Ready(Some(decoded.map(|d| (d, addr))))
        })
        .await
    }

    pub async fn recv_frame(&mut self) -> Option<Result<BytesMut, ChannelStreamError<E::Error>>> {
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
    ) -> Option<Result<(BytesMut, SocketAddr), ChannelStreamError<E::Error>>> {
        poll_fn(|cx| {
            let (frame, addr) = match ready!(self.stream_pool.poll_next_unpin(cx)) {
                Some((Ok(frame), addr)) => (frame, addr),
                Some((Err(error), addr)) => {
                    return Poll::Ready(Some(Err(error).context(PollNextSnafu { addr })))
                }
                None => return Poll::Ready(None),
            };

            Poll::Ready(Some(Ok((frame, addr))))
        })
        .await
    }
}

impl<'pin, T, E: DecodeMethod<T>, const N: usize, L: Accept> Stream for ChannelRef<'pin, T, E, N, L>
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

impl<'pin, T, E, const N: usize, L> ChannelRef<'pin, T, E, N, L>
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

impl<'pin, T: Clone, E: EncodeMethod<T>, const N: usize, L: Accept> Sink<T>
    for ChannelRef<'pin, T, E, N, L>
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

#[derive(Debug)]
pub struct ListenerRef<'pin, L>
where
    L: Accept,
{
    listener: &'pin mut L,
    stream_config: &'pin mut L::Config,
    done: bool,
}

impl<'pin, L> Drop for ListenerRef<'pin, L>
where
    L: Accept,
{
    fn drop(&mut self) {
        if !self.done {
            self.handle_abrupt_drop();
        }
    }
}

impl<'pin, L> ListenerRef<'pin, L>
where
    L: Accept,
{
    fn new(listener: &'pin mut L, stream_config: &'pin mut L::Config) -> Self {
        Self {
            listener,
            stream_config,
            done: false,
        }
    }

    fn config(&self) -> &L::Config {
        &self.stream_config
    }

    pub fn mark_done(&mut self) {
        self.done = true;
    }
}

impl<'pin, L> Accept for ListenerRef<'pin, L>
where
    L: Accept,
{
    type Output = L::Output;
    type Config = L::Config;
    type Error = L::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        self.listener.poll_accept(config, cx)
    }

    fn handle_abrupt_drop(&self) {
        self.listener.handle_abrupt_drop()
    }
}

#[derive(Debug)]
#[pin_project]
pub struct AcceptFuture<'pin, T, E, const N: usize, L>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
}

impl<'pin, T, E, const N: usize, L> AcceptFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    pub fn new(channel: &'pin mut Channel<T, E, N, L>) -> Self {
        Self { channel }
    }

    pub fn until<P, Until, Fut, O>(
        self,
        capture: P,
        until: Until,
    ) -> UntilAcceptFuture<'pin, T, E, N, L, Fut, O>
    where
        Until: Fn(ChannelRef<'pin, T, E, N, L>, P) -> Fut,
        Fut: Future<Output = (O, ChannelRef<'pin, T, E, N, L>)>,
    {
        let Channel {
            listener,
            local_addr,
            stream_pool,
            stream_config,
            ..
        } = self.channel;

        let limit = stream_pool.limit();

        let listener = ListenerRef::new(listener, stream_config);

        let channel = ChannelRef::new(stream_pool, *local_addr);

        UntilAcceptFuture::new(listener, limit, until(channel, capture))
    }
}

impl<'pin, T, E, const N: usize, L> Future for AcceptFuture<'pin, T, E, N, L>
where
    L: Accept,
{
    type Output = Result<SocketAddr, AcceptingError<L::Error>>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();

        let (s, a) = ready!(this
            .channel
            .listener
            .poll_accept(&this.channel.stream_config, cx))
        .context(AcceptingSnafu)?;

        this.channel
            .stream_pool
            .push_stream(s, a)
            .context(PushStreamSnafu)?;

        Poll::Ready(Ok(a))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct UntilAcceptFuture<'pin, T, E, const N: usize, L, Fut, O>
where
    L: Accept,
    L::Output: 'pin,
    Fut: Future<Output = (O, ChannelRef<'pin, T, E, N, L>)>,
{
    listener: ListenerRef<'pin, L>,
    limit: Option<usize>,
    accepts: Vec<SocketAddr>,
    streams: Vec<(L::Output, SocketAddr)>,
    should_accept: bool,
    #[pin]
    fut: Fut,
    _phantom: PhantomData<(T, E)>,
}

impl<'pin, T, E, const N: usize, L, Fut, O> UntilAcceptFuture<'pin, T, E, N, L, Fut, O>
where
    L: Accept,
    Fut: Future<Output = (O, ChannelRef<'pin, T, E, N, L>)>,
{
    pub fn new(
        listener: ListenerRef<'pin, L>,
        limit: Option<usize>,
        fut: Fut,
    ) -> UntilAcceptFuture<T, E, N, L, Fut, O> {
        Self {
            listener,
            limit,
            accepts: Vec::new(),
            streams: Vec::new(),
            should_accept: true,
            fut,
            _phantom: PhantomData, // channel,
        }
    }

    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(lim) = self.limit {
            self.limit = Some(lim.min(limit));
        } else {
            self.limit.replace(limit);
        }
        self
    }
}

impl<'pin, T, E, const N: usize, L, Fut, O> Future for UntilAcceptFuture<'pin, T, E, N, L, Fut, O>
where
    L: Accept,
    Fut: Future<Output = (O, ChannelRef<'pin, T, E, N, L>)>,
{
    type Output = (O, Vec<SocketAddr>);

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();

        // if there is a limit, and if there is room til the limit,
        // then poll accept again
        let should_accept = *this.should_accept
            && this
                .limit
                .map(|lim| this.streams.len() < lim)
                .unwrap_or(true);

        if should_accept {
            if let Poll::Ready(res) = this.listener.poll_accept(this.listener.config(), cx) {
                match res {
                    Ok((s, a)) => {
                        this.streams.push((s, a));
                        this.accepts.push(a);
                    }
                    Err(e) => {
                        *this.should_accept = false;
                        // this.accepts.push(Err(e).context(AcceptingSnafu));
                    }
                }
            }
        }

        let (o, ch) = ready!(this.fut.poll(cx));

        for (s, a) in this.streams.drain(..) {
            ch.stream_pool
                .push_stream(s, a)
                .expect("Only streams within limit were pushed");
        }

        Poll::Ready((o, this.accepts.drain(..).collect()))
    }
}
