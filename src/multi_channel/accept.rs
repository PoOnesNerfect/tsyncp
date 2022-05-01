use super::{errors::AcceptingError, AcceptingSnafu, Channel, PushStreamSnafu};
use crate::util::accept::Accept;
use futures::{ready, Future, FutureExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{marker::PhantomData, net::SocketAddr, task::Poll};

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
pub struct WhileAcceptingFuture<'pin, T, E, const N: usize, L, Fut>
where
    L: Accept,
    Fut: AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    #[pin]
    fut: Fut,
    limit: Option<usize>,
    addrs: Vec<SocketAddr>,
    err: Option<AcceptingError<L::Error>>,
    _phantom: PhantomData<(&'pin (), T, E, L)>,
}

impl<'pin, T, E, const N: usize, L, Fut> WhileAcceptingFuture<'pin, T, E, N, L, Fut>
where
    L: Accept,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    pub(super) fn new(fut: Fut) -> Self {
        let limit = fut.as_ref().limit();

        Self {
            fut,
            limit,
            addrs: Vec::new(),
            err: None,
            _phantom: PhantomData,
        }
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = self.limit.map(|lim| lim.min(limit));

        self
    }

    fn should_accept(&self) -> bool {
        let channel_ref = self.fut.as_ref();

        self.err.is_none()
            && self
                .limit
                .map(|lim| channel_ref.len() < lim)
                .unwrap_or(true)
    }
}

impl<'pin, T, E, const N: usize, L, Fut> Future for WhileAcceptingFuture<'pin, T, E, N, L, Fut>
where
    L: Accept,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    type Output = (
        Fut::Output,
        Result<Vec<SocketAddr>, AcceptingError<L::Error>>,
    );

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let channel_ref = self.fut.as_ref();

        if self.should_accept() {
            if let Poll::Ready(res) = channel_ref
                .listener
                .poll_accept(&channel_ref.stream_config, cx)
            {
                match res.context(AcceptingSnafu) {
                    Ok((s, a)) => {
                        let channel_mut = self.fut.as_mut();
                        channel_mut
                            .stream_pool
                            .push_stream(s, a)
                            .expect("Len is within limit");
                        self.addrs.push(a);
                    }
                    Err(e) => {
                        self.err.replace(e);
                    }
                }
            }
        }

        let output = ready!(self.fut.poll_unpin(cx));
        let addrs = if self.err.is_none() {
            Ok(self.addrs.drain(..).collect())
        } else {
            Err(self.err.take().unwrap())
        };

        // in case accepting stops without polling through
        self.fut.as_mut().listener.handle_abrupt_drop();

        Poll::Ready((output, addrs))
    }
}
