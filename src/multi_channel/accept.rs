//! Contains [AcceptFuture](AcceptFuture) which accepts and connection, and [ChainedAcceptFuture](ChainedAcceptFuture),
//! which is chain future to [SendFuture](super::send::SendFuture) and [RecvFuture](super::recv::RecvFuture) to
//! concurrently accept connections.

use super::{
    errors::{AcceptError, StreamPoolAcceptSnafu},
    Channel,
};
use crate::util::Accept;
use futures::{ready, Future, FutureExt};
use pin_project::pin_project;
use snafu::ResultExt;
use std::{marker::PhantomData, net::SocketAddr, task::Poll};

/// Future that accepts a connection, pushes the connection to connection pool, and returns the
/// remote address.
#[derive(Debug)]
#[pin_project]
pub struct AcceptFuture<'pin, T, E, const N: usize, L, H, F>
where
    L: Accept,
{
    channel: &'pin mut Channel<T, E, N, L>,
    num: usize,
    accepted: usize,
    to_limit: bool,
    handle: H,
    filter: F,
}

impl<'pin, T, E, const N: usize, L, H, F> AsRef<Channel<T, E, N, L>>
    for AcceptFuture<'pin, T, E, N, L, H, F>
where
    L: Accept,
{
    fn as_ref(&self) -> &Channel<T, E, N, L> {
        &self.channel
    }
}

impl<'pin, T, E, const N: usize, L, H, F> AsRef<usize> for AcceptFuture<'pin, T, E, N, L, H, F>
where
    L: Accept,
{
    fn as_ref(&self) -> &usize {
        &self.accepted
    }
}

impl<'pin, T, E, const N: usize, L, H, F> AcceptFuture<'pin, T, E, N, L, H, F>
where
    L: Accept,
{
    pub(super) fn new(channel: &'pin mut Channel<T, E, N, L>, handle: H, filter: F) -> Self {
        let num = channel.is_full().then(|| 0).unwrap_or(1);

        Self {
            num,
            accepted: 0,
            to_limit: false,
            handle,
            filter,
            channel,
        }
    }

    pub fn num(mut self, num: usize) -> Self {
        self.num = self
            .channel
            .limit()
            .map(|lim| num.min(lim - self.channel.len()))
            .unwrap_or(num);

        self.to_limit = false;

        self
    }

    pub fn to_limit(mut self) -> Self {
        self.num = self
            .channel
            .limit()
            .map(|lim| lim - self.channel.len())
            .unwrap_or(self.num);

        self.to_limit = true;

        self
    }

    pub fn handle<H2>(self, handle: H2) -> AcceptFuture<'pin, T, E, N, L, H2, F>
    where
        H2: Fn(SocketAddr),
    {
        let Self {
            channel,
            accepted,
            num,
            to_limit,
            filter,
            ..
        } = self;

        AcceptFuture {
            channel,
            accepted,
            num,
            to_limit,
            handle,
            filter,
        }
    }

    pub fn filter<F2>(self, filter: F2) -> AcceptFuture<'pin, T, E, N, L, H, F2>
    where
        F2: Fn(SocketAddr) -> bool,
    {
        let Self {
            channel,
            num,
            accepted,
            to_limit,
            handle,
            ..
        } = self;

        AcceptFuture {
            channel,
            num,
            accepted,
            to_limit,
            handle,
            filter,
        }
    }

    pub fn until<U>(mut self, until: U) -> UntilAcceptFuture<T, E, N, L, Self, U>
    where
        L: Accept,
        H: FnMut(SocketAddr),
        F: FnMut(SocketAddr) -> bool,
        U: Future,
    {
        if let Some(limit) = self.channel.limit() {
            if self.to_limit || self.num <= 1 {
                self.num = limit - self.channel.len();
            } else {
                self.num = self.num.min(limit - self.channel.len());
            }
        } else {
            // equivalent to no accept forever.
            self.num = usize::MAX;
        }

        UntilAcceptFuture::new(self, until)
    }
}

impl<'pin, T, E, const N: usize, L, H, F> Future for AcceptFuture<'pin, T, E, N, L, H, F>
where
    L: Accept,
    H: FnMut(SocketAddr),
    F: FnMut(SocketAddr) -> bool,
{
    type Output = Result<usize, AcceptError<L::Error>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        while self.accepted < self.num {
            match ready!(self
                .channel
                .listener
                .poll_accept(&self.channel.stream_config, cx))
            {
                Ok((stream, addr)) => {
                    if (self.filter)(addr) {
                        self.channel
                            .stream_pool
                            .push_stream(stream, addr)
                            .expect("limit is checked above");

                        (self.handle)(addr);

                        self.accepted += 1;
                    }
                }
                Err(err) => return Poll::Ready(Err(err).context(StreamPoolAcceptSnafu)),
            }
        }

        Poll::Ready(Ok(self.accepted))
    }
}

#[derive(Debug)]
#[pin_project]
pub struct UntilAcceptFuture<T, E, const N: usize, L, AFut, Fut>
where
    Fut: Future,
    AFut: Future,
{
    accept: AFut,
    accept_res: Option<AFut::Output>,
    #[pin]
    fut: Fut,
    _phantom: PhantomData<(T, E, L)>,
}

impl<T, E, const N: usize, L, AFut, Fut> UntilAcceptFuture<T, E, N, L, AFut, Fut>
where
    Fut: Future,
    AFut: Future,
{
    fn new(accept: AFut, fut: Fut) -> Self {
        Self {
            accept,
            accept_res: None,
            fut,
            _phantom: PhantomData,
        }
    }
}

impl<T, E, const N: usize, L, AFut, Fut> Future for UntilAcceptFuture<T, E, N, L, AFut, Fut>
where
    AFut: AsRef<Channel<T, E, N, L>>
        + AsRef<usize>
        + Future<Output = Result<usize, AcceptError<L::Error>>>
        + Unpin,
    Fut: Future,
    L: Accept,
{
    type Output = (Result<usize, AcceptError<L::Error>>, Fut::Output);

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.accept_res.is_none() {
            if let Poll::Ready(res) = this.accept.poll_unpin(cx) {
                this.accept_res.replace(res);
            }
        }

        let ret = ready!(this.fut.poll(cx));

        if this.accept_res.is_none() {
            let channel: &Channel<T, E, N, L> = this.accept.as_ref();
            channel.listener.handle_abrupt_drop();
        }

        Poll::Ready((
            this.accept_res.take().unwrap_or(Ok(*this.accept.as_ref())),
            ret,
        ))
    }
}

/// Future that polls accept concurrently as the future underneath.
///
/// This future completes immediately when the underlying future completes, regardless of if
/// polling accept completed or not.
///
/// ```no_run
/// use color_eyre::Result;
/// use serde::{Serialize, Deserialize};
/// use tsyncp::multi_channel;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct Dummy {
///     field1: String,
///     field2: u64,
///     field3: Vec<u8>,
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
///         .accept(10)
///         .await?;
///
///     if let (Some(Ok(item)), Ok(())) = ch.recv().accepting().await {
///         println!("{item:?} received");
///     }
///
///     Ok(())
/// }
/// ```
#[derive(Debug)]
#[pin_project]
pub struct ChainedAcceptFuture<'pin, T, E, const N: usize, L, Fut, H, F>
where
    L: Accept,
{
    #[pin]
    fut: Fut,
    num: Option<usize>,
    accepted: usize,
    err: Option<AcceptError<L::Error>>,
    handle: H,
    filter: F,
    _phantom: PhantomData<(&'pin (), T, E, L)>,
}

impl<'pin, T, E, const N: usize, L, Fut, H, F> ChainedAcceptFuture<'pin, T, E, N, L, Fut, H, F>
where
    L: Accept,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
{
    pub(super) fn new(fut: Fut, handle: H, filter: F) -> Self {
        let ch = fut.as_ref();
        let len = ch.len();
        let limit = ch.limit();

        Self {
            fut,
            num: limit.map(|lim| lim - len),
            accepted: 0,
            err: None,
            handle,
            filter,
            _phantom: PhantomData,
        }
    }

    /// Set the num for accepting connections.
    ///
    /// If the num given is higher than the Channel's set num, it will just use the Channel's
    /// num.
    ///
    /// ```no_run
    /// use color_eyre::Result;
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::multi_channel;
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy {
    ///     field1: String,
    ///     field2: u64,
    ///     field3: Vec<u8>,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let mut ch: multi_channel::JsonChannel<Dummy> = multi_channel::channel_on("localhost:11114")
    ///         .accept(10)
    ///         .await?;
    ///
    ///     // only accept up to 5 connections for this future.
    ///     if let (Some(Ok(item)), Ok(())) = ch.recv().accepting().num(5).await {
    ///         println!("{item:?} received");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn num(mut self, num: usize) -> Self {
        self.num
            .replace(self.num.map(|n| n.min(num)).unwrap_or(num));

        self
    }

    pub fn to_limit(mut self) -> Self {
        let ch = self.fut.as_ref();
        let len = ch.len();
        let limit = ch.limit();
        self.num = limit.map(|lim| lim - len);

        self
    }

    fn should_accept(&self) -> bool {
        self.err.is_none() && self.num.map(|num| self.accepted < num).unwrap_or(true)
    }

    pub fn handle<H2>(self, handle: H2) -> ChainedAcceptFuture<'pin, T, E, N, L, Fut, H2, F>
    where
        H2: FnMut(SocketAddr),
    {
        let Self {
            fut,
            accepted,
            num,
            err,
            filter,
            ..
        } = self;

        ChainedAcceptFuture {
            fut,
            num,
            accepted,
            err,
            handle,
            filter,
            _phantom: PhantomData,
        }
    }

    pub fn filter<F2>(self, filter: F2) -> ChainedAcceptFuture<'pin, T, E, N, L, Fut, H, F2>
    where
        F2: FnMut(SocketAddr) -> bool,
    {
        let Self {
            fut,
            num,
            accepted,
            err,
            handle,
            ..
        } = self;

        ChainedAcceptFuture {
            fut,
            num,
            accepted,
            err,
            handle,
            filter,
            _phantom: PhantomData,
        }
    }
}

impl<'pin, T, E, const N: usize, L, Fut, H, F> Future
    for ChainedAcceptFuture<'pin, T, E, N, L, Fut, H, F>
where
    L: Accept,
    Fut: AsRef<Channel<T, E, N, L>> + AsMut<Channel<T, E, N, L>> + Future + Unpin,
    H: FnMut(SocketAddr),
    F: FnMut(SocketAddr) -> bool,
{
    type Output = (Fut::Output, Result<usize, AcceptError<L::Error>>);

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        while self.should_accept() {
            let channel_ref = self.fut.as_ref();

            let poll = channel_ref
                .listener
                .poll_accept(&channel_ref.stream_config, cx);

            if let Poll::Ready(res) = poll {
                match res.context(StreamPoolAcceptSnafu) {
                    Ok((s, a)) => {
                        if (self.filter)(a) {
                            self.fut
                                .as_mut()
                                .stream_pool
                                .push_stream(s, a)
                                .expect("limit is checked above");

                            (self.handle)(a);

                            self.accepted += 1;
                        }
                    }
                    Err(e) => {
                        self.err.replace(e);
                        break;
                    }
                }
            } else {
                break;
            }
        }

        let output = ready!(self.fut.poll_unpin(cx));

        let accept_res = if self.err.is_none() {
            Ok(self.accepted)
        } else {
            Err(self.err.take().unwrap())
        };

        // in case accepting stops without polling through
        self.fut.as_mut().listener.handle_abrupt_drop();

        Poll::Ready((output, accept_res))
    }
}
