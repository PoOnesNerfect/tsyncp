//! Contains [StreamPool], which listens from all connections, and broadcasts to all connections.

use super::{frame_codec::VariedLengthDelimitedCodec, Framed, Split};
use bytes::{Bytes, BytesMut};
use futures::{ready, SinkExt, StreamExt};
use futures::{sink::Sink, stream::Stream};
use pin_project::pin_project;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::FramedParts;

type Result<T, E = StreamPoolError> = std::result::Result<T, E>;

/// Pool of byte streams (i.e. TcpStream), which manages receiving and send to the
/// underlying connections.
///
/// First generic parameter `S` is the type of the underlying byte stream, and const generic `N` is
/// the number of streams when the pool uses an array instead of a vec. When vec is used, `N`
/// defaults to `0`.
///
/// The pool can be made of either a [Vec] of streams, or an array of streams.
/// When it's an array, it stores the streams in an array on the stack, which may allow better
/// performance. Array's length is the limit of total number of connections the pool can have, and
/// current number of streams is kept as a field `len`.
/// Unused array indices are placed as uninitialized `MaybeUninit` elements.
/// This implementation does a careful job so that uninitialized memory is never accessed; however,
/// I do understand that part of the safety concern of using uninitialized memory is that, in the
/// future, I may forget what part of the code does what and accidently expose the unsafe code.
///
/// Therefore, if I find that using an array really has no effect on performance, I will remove
/// the array implementation completely.
///
/// For streaming data from connections, the pool tries to poll every stream in the pool at every
/// poll event. However, if one of the streams returns a result, it will save the last index of the
/// stream it was polled, and start polling at the next index in the next poll event.
///
/// For sending data to connections, it tries to send to all connections even when encountering an
/// error with *some* connections. So, it collects all the errors while sending data, and return
/// them after data is flushed. When an error is encountered, the pool removes the errored stream
/// from the pool, stores the error, then continues polling to the next stream.
#[derive(Debug)]
pub struct StreamPool<S, const N: usize = 0> {
    pool: Pool<S, N>,
    // Used for keeping track of current polling stream/sink stream index.
    // Since stream/sink never happens at the same time, it's safe to use the same field for both.
    index: usize,
    sink_errors: Vec<PollError>,
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
    /// Wrap the given byte stream into [Framed](crate::util::Framed), then push it into the pool.
    pub fn push_stream(&mut self, stream: S, addr: SocketAddr) -> Result<()> {
        let parts = FramedParts::new(stream, VariedLengthDelimitedCodec::new());

        self.push(Framed::from_parts(parts), addr)
    }
}

/// Splits the pool into pool of read-only streams, and pool of write-only streams.
impl<S, const N: usize> Split for StreamPool<S, N>
where
    S: Split,
    S::Error: 'static,
{
    type Left = StreamPool<S::Left, N>;
    type Right = StreamPool<S::Right, N>;
    type Error = UnsplitError<S::Error>;

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
        if is_array != w_half.is_array() {
            return Err(UnsplitError::RWUnequal);
        }

        let len = r_half.len();
        if len != w_half.len() {
            return Err(UnsplitError::RWUnequal);
        }

        let limit = r_half.limit();
        if limit != w_half.limit() {
            return Err(UnsplitError::RWUnequal);
        }

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
                let unsplit = Framed::<S>::unsplit(r, w).toss_unsplit()?;
                rw.push(unsplit, addr).expect("len should be sufficient");
            } else {
                rmap.insert(addr, r);
            }

            let (w, addr) = w_half.pop().expect("element should exist for popping");
            if let Some(r) = rmap.remove(&addr) {
                let unsplit = Framed::<S>::unsplit(r, w).toss_unsplit()?;
                rw.push(unsplit, addr).expect("len should be sufficient");
            } else {
                wmap.insert(addr, w);
            }
        }

        Ok(rw)
    }
}

impl<S, const N: usize> StreamPool<S, N> {
    /// Returns `true` is this pool uses an array to store streams.
    pub fn is_array(&self) -> bool {
        matches!(self.pool, Pool::Array { .. })
    }

    /// Returns `true` is this pool uses a Vec to store streams.
    pub fn is_vec(&self) -> bool {
        !self.is_array()
    }

    /// Returns pool with an array of streams with limit `M`.
    ///
    /// Attempt to push a stream when the pool already has `M` streams will return an error.
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

    /// Returns pool with a vec of streams with no limit or capacity.
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

    /// Returns a pool with a vec of streams with a capacity but without limit.
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

    /// Returns a pool with a vec of streams with a limit.
    ///
    /// Attempt to push a stream when the pool already has already reached the limit will return an error.
    pub fn with_limit(limit: usize) -> Self {
        let pool = Pool::Vec {
            streams: Vec::with_capacity(limit),
            limit: Some(limit),
        };

        StreamPool {
            pool,
            index: 0,
            sink_errors: Vec::new(),
        }
    }

    /// Returns current number of streams in the pool.
    pub fn len(&self) -> usize {
        match self.pool {
            Pool::Array { len, .. } => len,
            Pool::Vec { ref streams, .. } => streams.len(),
        }
    }

    /// Returns total number of streams available in the pool.
    pub fn limit(&self) -> Option<usize> {
        match self.pool {
            Pool::Array { .. } => Some(N),
            Pool::Vec { limit, .. } => limit,
        }
    }

    /// Returns remote addresses of all streams in the pool.
    pub fn addrs(&self) -> Vec<SocketAddr> {
        let len = self.len();
        let mut addrs = Vec::with_capacity(len);

        for i in 0..len {
            addrs.push(self.get(i).expect("Element should exist with len").1);
        }

        addrs
    }

    /// Returns a reference of stream and address at the index.
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

    /// Returns a mutable reference of stream and address at the index.
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

    /// Push a stream into the pool.
    ///
    /// This will return an error if attempted to push a stream while pool is full.
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
                        return Err(StreamPoolError::AddStreamLimitReached { limit: N });
                    }
                }

                streams.push((stream, addr));
            }
        }

        Ok(())
    }

    /// Pops and returns a stream from the pool.
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

    /// Swaps a stream at the index with the last element, and pops that element.
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

    pub(crate) fn drain_sink_res(&mut self) -> Result<(), SinkErrors> {
        let errors = self.sink_errors.drain(..).collect::<Vec<_>>();

        // have first error as source
        if !errors.is_empty() {
            Err(SinkErrors::new(errors))
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
    type Item = (Result<BytesMut, PollError>, SocketAddr);

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

                    return Poll::Ready(Some((item.toss_poll_next(addr), addr)));
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

    pub(crate) fn poll_ready_indices(
        &mut self,
        indices: &mut Vec<usize>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<()> {
        // Go in reverse order,
        // so that, when we encounter an error,
        // we can swap_remove it and... just flush it away.
        while let Some(i) = indices.last() {
            let (stream, addr) = self.get_mut(*i).expect("Element should exist with index");
            if let Err(e) = ready!(stream.poll_ready_unpin(cx)).toss_poll_ready(*addr) {
                self.swap_remove(*i);
                self.sink_errors.push(e);
            }

            // Ready or not, this index is not pending anymore.
            indices.pop();
        }

        Poll::Ready(())
    }

    pub(crate) fn start_send_filter<Filter: Fn(SocketAddr) -> bool>(
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
            let (stream, addr) = self.get_mut(i).expect("Element should exist with index");

            // Gotta go fast!
            // Errors never bothered me anyway.
            if let Err(e) = stream.start_send_unpin(item.clone()).toss_start_send(*addr) {
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
        // Go in reverse order,
        // so that, when we encounter an error,
        // we can swap_remove it and... just flush it away.
        while let Some(i) = indices.last() {
            let (stream, addr) = self.get_mut(*i).expect("Element should exist with index");
            if let Err(e) = ready!(stream.poll_flush_unpin(cx)).toss_poll_flush(*addr) {
                self.swap_remove(*i);
                self.sink_errors.push(e);
            }

            // Ready or not, this index is not pending anymore.
            indices.pop();
        }

        Poll::Ready(())
    }
}

impl<S: AsyncWrite + Unpin, const N: usize> StreamPool<S, N> {
    // poll on all streams in the pool, while skipping the ones that already returned ready.
    fn poll_all<F>(&mut self, mut f: F) -> Poll<Result<(), SinkErrors>>
    where
        F: FnMut(&mut Framed<S>, SocketAddr) -> Poll<Result<(), PollError>>,
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

            let (stream, addr) = (*self)
                .get_mut(i)
                .expect("stream should be initialized from 0..len");

            if let Err(error) = ready!(f(stream, *addr)) {
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
    type Error = SinkErrors;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.poll_all(|s, addr| {
            s.poll_ready_unpin(cx)
                .map_err(|e| PollError::PollReady { addr, source: e })
        })
    }

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: Bytes) -> Result<(), Self::Error> {
        let mut len = self.len();

        if len == 0 {
            return Ok(());
        }

        let send = |this: &mut Self, n, t| {
            let (stream, addr) = this
                .get_mut(n)
                .expect("stream should be initialized from 0..len");

            stream.start_send_unpin(t).toss_start_send(*addr)
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
        ready!(self.poll_all(|s, addr| s
            .poll_flush_unpin(cx)
            .map_err(|e| { PollError::PollFlush { addr, source: e } })))?;

        // Vomit all errors collected from `poll_ready`, `start_send` and `poll_flush`.
        Poll::Ready(self.drain_sink_res())
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        // Only returns error when len is 0, so must return sink_res again later.
        ready!(self.poll_all(|s, addr| s
            .poll_close_unpin(cx)
            .map_err(|e| { PollError::PollClose { addr, source: e } })))?;

        // After poll close, drain pool.
        while self.pop().is_some() {}

        // Vomit all collected errors.
        Poll::Ready(self.drain_sink_res())
    }
}

use crate::util::frame_codec;
use std::{
    error::Error,
    fmt, io,
    ops::{Deref, DerefMut},
};
use thiserror::Error;
use tosserror::Toss;

#[allow(missing_docs)]
#[derive(Debug, Error, Toss)]
#[visibility(pub(super))]
pub enum StreamPoolError {
    #[error("[StreamPoolError] Cannot add stream to pool as limit {limit} is already reached.")]
    AddStreamLimitReached { limit: usize },
}

#[allow(missing_docs)]
#[derive(Debug, Error, Toss)]
#[error("[SinkErrors] Encountered following errors while sending item:")]
#[visibility(pub(super))]
pub struct SinkErrors {
    source: SinkErrorsInner,
}

#[allow(missing_docs)]
#[derive(Debug)]
pub struct SinkErrorsInner {
    errors: Vec<PollError>,
}

impl Deref for SinkErrorsInner {
    type Target = Vec<PollError>;

    fn deref(&self) -> &Self::Target {
        &self.errors
    }
}

impl DerefMut for SinkErrorsInner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.errors
    }
}

impl std::error::Error for SinkErrorsInner {}

impl fmt::Display for SinkErrorsInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for err in &self.errors {
            write!(f, "\n- {}\n\n", err)?;
            write!(f, "  Caused by:\n")?;

            if let Some(source) = err.source() {
                let mut i = 0;
                let mut error = Some(source);

                while let Some(e) = error {
                    write!(f, "     {}: {}\n", i, e)?;

                    i += 1;
                    error = e.source();
                }
            }
        }

        Ok(())
    }
}

#[allow(missing_docs)]
impl SinkErrors {
    pub fn new(errors: Vec<PollError>) -> Self {
        Self {
            source: SinkErrorsInner { errors },
        }
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.iter().map(|e| e.peer_addr().to_owned()).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PollError> {
        self.source.errors.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = PollError> {
        self.source.errors.into_iter()
    }

    pub fn as_io_errors(&self) -> impl Iterator<Item = &std::io::Error> {
        self.iter().filter_map(|e| e.as_io())
    }

    pub fn into_io_errors(self) -> impl Iterator<Item = std::io::Error> {
        self.into_iter().filter_map(|e| e.into_io())
    }
}

#[allow(missing_docs)]
#[derive(Debug, Error, Toss)]
#[visibility(pub(super))]
pub enum UnsplitError<E>
where
    E: 'static + std::error::Error,
{
    #[error("[UnsplitError] Failed to unsplit underlying stream; read half and write half were not equal")]
    RWUnequal,
    #[error("[UnsplitError] Failed to unsplit underlying stream")]
    Unsplit { source: E },
}

#[allow(missing_docs)]
#[derive(Debug, Error, Toss)]
#[visibility(pub(super))]
pub enum PollError {
    #[error("[PollError] Error start sending to {addr}")]
    StartSend {
        addr: SocketAddr,
        source: frame_codec::CodecError,
    },
    #[error("[PollError] Error polling ready for {addr}")]
    PollReady {
        addr: SocketAddr,
        source: frame_codec::CodecError,
    },
    #[error("[PollError] Error flushing buffer to {addr}")]
    PollFlush {
        addr: SocketAddr,
        source: frame_codec::CodecError,
    },
    #[error("[PollError] Error closing connection to {addr}")]
    PollClose {
        addr: SocketAddr,
        source: frame_codec::CodecError,
    },
    #[error("[StreamPoolError] Failed poll_next in stream_pool")]
    PollNext {
        addr: SocketAddr,
        source: frame_codec::CodecError,
    },
}

#[allow(missing_docs)]
impl PollError {
    pub fn peer_addr(&self) -> &SocketAddr {
        match self {
            Self::StartSend { addr, .. } => addr,
            Self::PollReady { addr, .. } => addr,
            Self::PollFlush { addr, .. } => addr,
            Self::PollClose { addr, .. } => addr,
            Self::PollNext { addr, .. } => addr,
        }
    }

    pub fn as_io(&self) -> Option<&std::io::Error> {
        match self {
            Self::StartSend { source, .. } => source.as_io(),
            Self::PollReady { source, .. } => source.as_io(),
            Self::PollFlush { source, .. } => source.as_io(),
            Self::PollClose { source, .. } => source.as_io(),
            Self::PollNext { source, .. } => source.as_io(),
        }
    }

    pub fn into_io(self) -> Option<std::io::Error> {
        match self {
            Self::StartSend { source, .. } => source.into_io(),
            Self::PollReady { source, .. } => source.into_io(),
            Self::PollFlush { source, .. } => source.into_io(),
            Self::PollClose { source, .. } => source.into_io(),
            Self::PollNext { source, .. } => source.into_io(),
        }
    }

    /// Check if the error is a connection error.
    ///
    /// Returns `true` if the error either `reset`, `refused`, `aborted`, `not connected`, or
    /// `broken pipe`.
    ///
    /// This is useful to see if the returned error is from the underlying TCP connection.
    /// This method will be bubbled up with the error, and also be available at the highest
    /// level.
    pub fn is_connection_error(&self) -> bool {
        self.is_connection_reset()
            || self.is_connection_refused()
            || self.is_connection_aborted()
            || self.is_not_connected()
            || self.is_broken_pipe()
    }

    pub fn is_connection_reset(&self) -> bool {
        self.as_io()
            .map(|e| e.kind() == io::ErrorKind::ConnectionReset)
            .unwrap_or_default()
    }

    pub fn is_connection_refused(&self) -> bool {
        self.as_io()
            .map(|e| e.kind() == io::ErrorKind::ConnectionRefused)
            .unwrap_or_default()
    }

    pub fn is_connection_aborted(&self) -> bool {
        self.as_io()
            .map(|e| e.kind() == io::ErrorKind::ConnectionAborted)
            .unwrap_or_default()
    }

    pub fn is_not_connected(&self) -> bool {
        self.as_io()
            .map(|e| e.kind() == io::ErrorKind::NotConnected)
            .unwrap_or_default()
    }

    pub fn is_broken_pipe(&self) -> bool {
        self.as_io()
            .map(|e| e.kind() == io::ErrorKind::BrokenPipe)
            .unwrap_or_default()
    }
}
