use super::frame_codec::{errors::LengthDelimitedCodecError, VariedLengthDelimitedCodec};
use super::split::{errors::SplitError, Split};
use super::Framed;
use bytes::{Bytes, BytesMut};
use errors::*;
use futures::future::poll_fn;
use futures::{ready, SinkExt, StreamExt};
use futures::{sink::Sink, stream::Stream};
use pin_project::pin_project;
use snafu::{ensure, Backtrace, ResultExt};
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::FramedParts;

type Result<T, E = StreamPoolError> = std::result::Result<T, E>;

#[derive(Debug)]
pub struct StreamPool<S, const N: usize = 0> {
    pool: Pool<S, N>,
    // Used for keeping track of current polling stream/sink stream index.
    // Since stream/sink never happens at the same time, it's safe to use the same field for both.
    index: usize,
    sink_errors: Vec<StreamPoolPollError>,
}

#[derive(Debug)]
#[pin_project(project = StreamPoolProj)]
enum Pool<S, const N: usize = 0> {
    Array {
        streams: [MaybeUninit<(Framed<S>, SocketAddr)>; N],
        len: usize,
    },
    Vec {
        streams: Vec<(Framed<S>, SocketAddr)>,
        limit: Option<usize>,
    },
}

impl<S, const N: usize> StreamPool<S, N> {
    pub fn push_stream(&mut self, stream: S, addr: SocketAddr) -> Result<()> {
        let parts = FramedParts::new(stream, VariedLengthDelimitedCodec::new());

        self.push(Framed::from_parts(parts), addr)
    }
}

impl<S, const N: usize> Split for StreamPool<S, N>
where
    S: Split,
    S::Error: 'static,
{
    type Left = StreamPool<S::Left, N>;
    type Right = StreamPool<S::Right, N>;
    type Error = StreamPoolSplitError<S::Error>;

    fn split(mut self) -> (Self::Left, Self::Right) {
        let is_array = self.is_array();
        let limit = self.limit();
        let len = self.len();

        let mut left_streams = Vec::with_capacity(len);
        let mut right_streams = Vec::with_capacity(len);
        let mut addrs = Vec::with_capacity(len);

        for _ in 0..len {
            let (rw, addr) = self.pop().expect("element should exist for popping");
            let (r, w) = rw.split();

            left_streams.push(r);
            right_streams.push(w);
            addrs.push(addr);
        }

        let (mut left, mut right) = if is_array {
            (StreamPool::<_, N>::array(), StreamPool::<_, N>::array())
        } else if let Some(limit) = limit {
            (StreamPool::with_limit(limit), StreamPool::with_limit(limit))
        } else {
            (
                StreamPool::with_capacity(len),
                StreamPool::with_capacity(len),
            )
        };

        for ((r, w), addr) in left_streams
            .into_iter()
            .zip(right_streams.into_iter())
            .zip(addrs.into_iter())
        {
            left.push(r, addr).expect("len should be sufficient");
            right.push(w, addr).expect("len should be sufficient");
        }

        (left, right)
    }

    fn unsplit(mut r_half: Self::Left, mut w_half: Self::Right) -> Result<Self, Self::Error> {
        let is_array = r_half.is_array();
        ensure!(is_array == w_half.is_array(), RWUnequalSnafu);

        let len = r_half.len();
        ensure!(len == w_half.len(), RWUnequalSnafu);

        let limit = r_half.limit();
        ensure!(limit == w_half.limit(), RWUnequalSnafu);

        let mut rw = if is_array {
            Self::array()
        } else if let Some(limit) = limit {
            Self::with_limit(limit)
        } else {
            Self::with_capacity(len)
        };

        let mut rmap = std::collections::HashMap::with_capacity(len);
        let mut wmap = std::collections::HashMap::with_capacity(len);

        for _ in 0..len {
            let (r, addr) = r_half.pop().expect("element should exist for popping");
            if let Some(w) = wmap.remove(&addr) {
                let unsplit = Framed::<S>::unsplit(r, w).context(UnsplitSnafu)?;
                rw.push(unsplit, addr).expect("len should be sufficient");
            } else {
                rmap.insert(addr, r);
            }

            let (w, addr) = w_half.pop().expect("element should exist for popping");
            if let Some(r) = rmap.remove(&addr) {
                let unsplit = Framed::<S>::unsplit(r, w).context(UnsplitSnafu)?;
                rw.push(unsplit, addr).expect("len should be sufficient");
            } else {
                wmap.insert(addr, w);
            }
        }

        Ok(rw)
    }
}

impl<S, const N: usize> StreamPool<S, N> {
    pub fn is_array(&self) -> bool {
        matches!(self.pool, Pool::Array { .. })
    }

    pub fn is_vec(&self) -> bool {
        !self.is_array()
    }

    pub fn array<const M: usize>() -> StreamPool<S, M> {
        let pool = Pool::Array {
            streams: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
        };

        StreamPool {
            pool,
            index: 0,
            sink_errors: Vec::new(),
        }
    }

    pub fn vec() -> Self {
        let pool = Pool::Vec {
            streams: Vec::new(),
            limit: None,
        };

        StreamPool {
            pool,
            index: 0,
            sink_errors: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let pool = Pool::Vec {
            streams: Vec::with_capacity(capacity),
            limit: None,
        };

        StreamPool {
            pool,
            index: 0,
            sink_errors: Vec::new(),
        }
    }

    pub fn with_limit(capacity: usize) -> Self {
        let pool = Pool::Vec {
            streams: Vec::with_capacity(capacity),
            limit: Some(capacity),
        };

        StreamPool {
            pool,
            index: 0,
            sink_errors: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        match self.pool {
            Pool::Array { len, .. } => len,
            Pool::Vec { ref streams, .. } => streams.len(),
        }
    }

    pub fn limit(&self) -> Option<usize> {
        match self.pool {
            Pool::Array { .. } => Some(N),
            Pool::Vec { limit, .. } => limit,
        }
    }

    pub fn addrs(&self) -> Vec<SocketAddr> {
        let len = self.len();
        let mut addrs = Vec::with_capacity(len);

        for i in 0..len {
            addrs.push(self.get(i).expect("Element should exist with len").1);
        }

        addrs
    }

    pub fn get(&self, index: usize) -> Option<&(Framed<S>, SocketAddr)> {
        match &self.pool {
            Pool::Array { streams, len, .. } => {
                if index >= *len {
                    return None;
                }

                Some(unsafe { &*streams[index].as_ptr() })
            }
            Pool::Vec { streams, .. } => streams.get(index),
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut (Framed<S>, SocketAddr)> {
        match &mut self.pool {
            Pool::Array { streams, len, .. } => {
                if index >= *len {
                    return None;
                }

                Some(unsafe { &mut *streams[index].as_mut_ptr() })
            }
            Pool::Vec { streams, .. } => streams.get_mut(index),
        }
    }

    pub fn push(&mut self, stream: Framed<S>, addr: SocketAddr) -> Result<()> {
        match &mut self.pool {
            Pool::Array { streams, len, .. } => {
                if *len == N {
                    return Err(StreamPoolError::AddStreamLimitReached { limit: N });
                }

                streams[*len].write((stream, addr));
                *len += 1;
            }
            Pool::Vec { streams, limit, .. } => {
                if let Some(limit) = limit {
                    if streams.len() == *limit {
                        return Err(StreamPoolError::AddStreamLimitReached { limit: *limit });
                    }
                }

                streams.push((stream, addr));
            }
        }

        Ok(())
    }

    pub fn pop(&mut self) -> Option<(Framed<S>, SocketAddr)> {
        match &mut self.pool {
            Pool::Array { streams, len, .. } => {
                // if there are no elements, no actions required.
                if *len == 0 {
                    return None;
                }

                *len -= 1;

                // extract the stream to remove.
                //
                // This operation is safe if the following is true: if len > 0, then we know that streams[len - 1] is initialized;
                // and we know that this is true because the only way to increment `len` is via `push(..)` which initializes value at `len`.
                Some(unsafe {
                    std::mem::replace(&mut streams[*len], MaybeUninit::uninit()).assume_init()
                })
            }
            Pool::Vec { streams, .. } => streams.pop(),
        }
    }

    pub fn swap_remove(&mut self, index: usize) -> Option<(Framed<S>, SocketAddr)> {
        match &mut self.pool {
            Pool::Array { streams, len, .. } => {
                // if there are no elements, no action required.
                if *len == 0 {
                    return None;
                }

                // if index is greater or equal to number of elements, no action required.
                if index >= *len {
                    return None;
                }

                // if index is the last index, then pop last element.
                if index == *len - 1 {
                    return self.pop();
                }

                // swap element at index with last element.
                streams.swap(index, *len - 1);

                // pop last element.
                self.pop()
            }
            Pool::Vec { streams, .. } => {
                let streams_len = streams.len();

                // if there are no streams, no action required.
                if streams_len == 0 {
                    return None;
                }

                // if index is greater or equal to number of streams, no action required.
                if index >= streams_len {
                    return None;
                }

                // if index is the last index, then pop last element.
                if index == streams_len - 1 {
                    streams.pop();
                    return None;
                }

                // swap remove element at index.
                Some(streams.swap_remove(index))
            }
        }
    }

    pub(crate) fn drain_sink_res(&mut self) -> Result<(), StreamPoolSinkError> {
        let mut errors = self.sink_errors.drain(..);

        // have first error as source
        if let Some(source) = errors.next() {
            return Err(StreamPoolSinkError::new(
                errors.collect(),
                source,
                Backtrace::new(),
            ));
        } else {
            Ok(())
        }
    }
}

// Possible vulnerability:
// Let's say an item is ready and waker is called at index 6.
// However, when the executor polls this function again, an item at index 3 becomes also ready.
// At this point, instead of fetching and returning item at index 6, we return an item at index 3.
// And item at index 6 will not be polled and fetched again (assuming?) since its waker for the
// item was already called.
//
// Possible rebuttal:
// However, this will most likely never happen, because in situations where there are a lot of
// items coming in, the stream in question will likely be polled again; and, for situations where
// there are not many items coming in, it's unlikely that other stream will snatch its turn.
// Also, this implementation is round-robin, so it'll give every stream another chance to be polled
// at every polling event.
//
// Possible solution:
// Pass different wakers into different streams when polling, so we know which stream we can poll
// when it's called, instead of round-robin polling all streams.
impl<S: AsyncRead + Unpin, const N: usize> Stream for StreamPool<S, N> {
    type Item = (Result<BytesMut>, SocketAddr);

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let len = self.len();

        if len == 0 {
            return Poll::Ready(None);
        }

        let mut index = self.index % len;
        for _ in 0..len {
            let (stream, addr) = (*self)
                .get_mut(index)
                .expect("stream should be initialized from 0..len");

            match stream.poll_next_unpin(cx) {
                Poll::Ready(Some(item)) => {
                    let addr = *addr;

                    if item.is_err() {
                        let _ = self.swap_remove(index);

                        // since stream at this index is removed, start at the same index next poll
                        self.index = index;
                    } else {
                        // start poll at the next index for next poll
                        self.index = (index + 1) % len;
                    }

                    return Poll::Ready(Some((item.context(PollNextSnafu), addr)));
                }
                _ => {}
            }

            index = (index + 1) % len;
        }

        // next poll, start at next index
        self.index = index;
        Poll::Pending
    }
}

impl<S: AsyncWrite + Unpin, const N: usize> StreamPool<S, N> {
    pub(crate) fn poll_ready_indices(
        &mut self,
        indices: &mut Vec<usize>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<()> {
        self.poll_indices(indices, |s| s.poll_ready_unpin(cx))
    }

    pub(crate) fn start_send_filtered<Filter: Fn(SocketAddr) -> bool>(
        &mut self,
        item: Bytes,
        filter: &Filter,
    ) {
        let mut len = self.len();

        // Whatevs.... they were all trash connections anyway.
        if len == 0 {
            return;
        }

        // Go in reverse order,
        // so that, when we encounter an error,
        // we can just swap_remove it and let the past be past.
        let indices = (0..len)
            .filter(|i| {
                let (_stream, addr) = self.get(*i).expect("Element should exist with len");
                filter(*addr)
            })
            .rev()
            .collect::<Vec<_>>();

        for i in indices {
            let (stream, _) = self.get_mut(i).expect("Element should exist with index");

            // Gotta go fast!
            // Errors never bothered me anyway.
            if let Err(e) = stream
                .start_send_unpin(item.clone())
                .context(StartSendSnafu)
            {
                self.swap_remove(i);
                self.sink_errors.push(e);

                len -= 1;
            }
        }
    }

    pub(crate) fn poll_flush_indices(
        &mut self,
        indices: &mut Vec<usize>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<()> {
        self.poll_indices(indices, |s| s.poll_flush_unpin(cx))
    }
}

impl<S: AsyncWrite + Unpin, const N: usize> StreamPool<S, N> {
    pub(crate) fn get_indices<Filter>(&self, filter: &Filter) -> Vec<usize>
    where
        Filter: Fn(SocketAddr) -> bool,
    {
        (0..self.len())
            .filter(|i| {
                let (_stream, addr) = self.get(*i).expect("Element should exist with len");
                filter(*addr)
            })
            .collect::<Vec<_>>()
    }

    fn poll_indices<F>(&mut self, indices: &mut Vec<usize>, mut f: F) -> Poll<()>
    where
        F: FnMut(&mut Framed<S>) -> Poll<Result<(), LengthDelimitedCodecError>>,
    {
        // Go in reverse order,
        // so that, when we encounter an error,
        // we can swap_remove it and... just flush it away.
        while let Some(i) = indices.last() {
            let (stream, _) = self.get_mut(*i).expect("Element should exist with index");
            if let Err(e) = ready!(f(stream)).context(PollIndicesSnafu) {
                self.swap_remove(*i);
                self.sink_errors.push(e);
            }

            // Ready or not, this index is not pending anymore.
            indices.pop();
        }

        Poll::Ready(())
    }

    // poll on all streams in the pool, while skipping the ones that already returned ready.
    fn poll_all<F>(&mut self, mut f: F) -> Poll<Result<(), StreamPoolSinkError>>
    where
        F: FnMut(&mut Framed<S>) -> Poll<Result<(), LengthDelimitedCodecError>>,
    {
        let mut len = self.len();

        if len == 0 {
            // if there are no streams, reset index and return
            self.index = 0;
            return Poll::Ready(self.drain_sink_res());
        }

        // if we were finished, then start over!
        if self.index == 0 {
            // start from last index and move down
            self.index = len;
        }

        while self.index > 0 {
            let i = self.index - 1;

            let (stream, _) = (*self)
                .get_mut(i)
                .expect("stream should be initialized from 0..len");

            if let Err(error) = ready!(f(stream)).context(PollSinkSnafu) {
                self.swap_remove(i);
                self.sink_errors.push(error);

                len -= 1;
            }

            self.index -= 1;
        }

        // at this point, self.index == 0

        if len == 0 {
            Poll::Ready(self.drain_sink_res())
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

impl<S: AsyncWrite + Unpin, const N: usize> Sink<Bytes> for StreamPool<S, N> {
    type Error = StreamPoolSinkError;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.poll_all(|s| s.poll_ready_unpin(cx))
    }

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: Bytes) -> Result<(), Self::Error> {
        let mut len = self.len();

        if len == 0 {
            return Ok(());
        }

        let send = |this: &mut Self, n, t| {
            let (stream, _) = this
                .get_mut(n)
                .expect("stream should be initialized from 0..len");

            stream.start_send_unpin(t).context(StartSendSnafu)
        };

        for i in (0..len).rev() {
            if let Err(error) = send(&mut self, i, item.clone()) {
                self.swap_remove(i);
                self.sink_errors.push(error);

                len -= 1;
            }
        }

        if len == 0 {
            self.drain_sink_res()
        } else {
            Ok(())
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // Only returns error when len is 0, so must return sink_res again later.
        ready!(self.poll_all(|s| s.poll_flush_unpin(cx)))?;

        // Vomit all errors collected from `poll_ready`, `start_send` and `poll_flush`.
        Poll::Ready(self.drain_sink_res())
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // Only returns error when len is 0, so must return sink_res again later.
        ready!(self.poll_all(|s| s.poll_close_unpin(cx)))?;

        // After poll close, drain pool.
        while self.pop().is_some() {}

        // Vomit all collected errors.
        Poll::Ready(self.drain_sink_res())
    }
}

pub mod errors {
    use super::*;
    use snafu::Snafu;

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamPoolError {
        #[snafu(display(
            "[StreamPoolError] Cannot add stream to pool as limit {limit} is already reached."
        ))]
        AddStreamLimitReached { limit: usize },
        #[snafu(display("[StreamPoolError] Failed poll_next"))]
        PollNext {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolError] Failed to split underlying stream"))]
        Split {
            source: SplitError,
            backtrace: Backtrace,
        },
    }

    impl StreamPoolError {
        pub fn as_io(&self) -> Option<&std::io::Error> {
            if let Self::PollNext { source, .. } = self {
                return source.as_io();
            }

            None
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(display("[StreamPoolSinkError] Failed to send item"))]
    #[snafu(visibility(pub(super)))]
    pub struct StreamPoolSinkError {
        errors: Vec<StreamPoolPollError>,
        source: StreamPoolPollError,
        backtrace: Backtrace,
    }

    impl StreamPoolSinkError {
        pub fn new(
            errors: Vec<StreamPoolPollError>,
            source: StreamPoolPollError,
            backtrace: Backtrace,
        ) -> Self {
            Self {
                errors,
                source,
                backtrace,
            }
        }

        pub fn errors(&self) -> impl Iterator<Item = &StreamPoolPollError> {
            std::iter::once(&self.source).chain(self.errors.iter())
        }

        pub fn into_errors(self) -> impl Iterator<Item = StreamPoolPollError> {
            std::iter::once(self.source).chain(self.errors.into_iter())
        }

        pub fn as_io(&self) -> impl Iterator<Item = &std::io::Error> {
            std::iter::once(&self.source)
                .chain(self.errors.iter())
                .map(|e| e.as_io())
                .filter_map(|e| e)
        }

        pub fn into_io(self) -> impl Iterator<Item = std::io::Error> {
            std::iter::once(self.source)
                .chain(self.errors.into_iter())
                .map(|e| e.into_io())
                .filter_map(|e| e)
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamPoolSplitError<E>
    where
        E: 'static + snafu::Error,
    {
        #[snafu(display("[StreamPoolSplitError] Failed to unsplit underlying stream; read half and write half were not equal"))]
        RWUnequal,
        #[snafu(display("[StreamPoolSplitError] Failed to unsplit underlying stream"))]
        Unsplit { source: E, backtrace: Backtrace },
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamPoolPollError {
        #[snafu(display("[StreamPoolPollError] Error in start_send"))]
        StartSend {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolPollError] Error in poll_indices"))]
        PollIndices {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolPollError] Error in poll_all"))]
        PollSink {
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
    }

    impl StreamPoolPollError {
        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::StartSend { source, .. } => source.as_io(),
                Self::PollIndices { source, .. } => source.as_io(),
                Self::PollSink { source, .. } => source.as_io(),
            }
        }

        pub fn into_io(self) -> Option<std::io::Error> {
            match self {
                Self::StartSend { source, .. } => source.into_io(),
                Self::PollIndices { source, .. } => source.into_io(),
                Self::PollSink { source, .. } => source.into_io(),
            }
        }
    }
}
