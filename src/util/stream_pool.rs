use super::frame_codec::{errors::LengthDelimitedCodecError, VariedLengthDelimitedCodec};
use super::split::{self, errors::SplitError, RWSplit, TcpSplit};
use super::{Framed, TypeName};
use crate::util::split::unsplit_framed;
use bytes::{Bytes, BytesMut};
use errors::*;
use futures::future::poll_fn;
use futures::{ready, SinkExt, StreamExt};
use futures::{sink::Sink, stream::Stream};
use pin_project::pin_project;
use snafu::{ensure, Backtrace, Error, ResultExt};
use std::fmt;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::task::Poll;
use std::vec::Drain;
use tokio::io::{AsyncRead, AsyncWrite};

type Result<T, E = StreamPoolError> = std::result::Result<T, E>;

#[derive(Debug)]
pub struct StreamPool<S, const N: usize = 0> {
    pool: Pool<S, N>,
    stream_index: usize,
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

impl<const N: usize> StreamPool<TcpSplit, N> {
    pub(crate) fn push_stream(&mut self, stream: TcpSplit, addr: SocketAddr) -> Result<()> {
        let framed = Framed::new(stream, VariedLengthDelimitedCodec::new());

        self.push(framed, addr)
    }
}

impl<R, W, const N: usize> StreamPool<RWSplit<R, W>, N> {
    pub(crate) fn split(mut self) -> Result<(Self, Self), StreamPoolError> {
        let is_array = self.is_array();
        let limit = self.limit();

        let len = self.len();
        let mut rs = Vec::with_capacity(len);
        let mut ws = Vec::with_capacity(len);

        let mut addrs = Vec::with_capacity(len);

        for _ in 0..len {
            let (rw, addr) = self.pop().expect("element should exist for popping");
            let (r, w) = split::split_framed(rw).context(SplitSnafu)?;

            rs.push(r);
            ws.push(w);
            addrs.push(addr);
        }

        let (mut r_pool, mut w_pool) = if is_array {
            (Self::array(), Self::array())
        } else if let Some(limit) = limit {
            (Self::with_limit(limit), Self::with_limit(limit))
        } else {
            (Self::with_capacity(len), Self::with_capacity(len))
        };

        for ((r, w), addr) in rs.into_iter().zip(ws.into_iter()).zip(addrs.into_iter()) {
            r_pool.push(r, addr);
            w_pool.push(w, addr);
        }

        Ok((r_pool, w_pool))
    }

    pub(crate) fn unsplit(mut r_half: Self, mut w_half: Self) -> Result<Self, StreamPoolError> {
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

        let mut map = std::collections::HashMap::with_capacity(len);

        for _ in 0..len {
            let (r, addr) = r_half.pop().expect("element should exist for popping");
            if let Some(w) = map.remove(&addr) {
                let unsplit = unsplit_framed(r, w).context(UnsplitSnafu)?;
                rw.push(unsplit, addr);
            } else {
                map.insert(addr, r);
            }

            let (w, addr) = w_half.pop().expect("element should exist for popping");
            if let Some(r) = map.remove(&addr) {
                let unsplit = unsplit_framed(r, w).context(UnsplitSnafu)?;
                rw.push(unsplit, addr);
            } else {
                map.insert(addr, w);
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
            stream_index: 0,
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
            stream_index: 0,
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
            stream_index: 0,
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
            stream_index: 0,
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
                // and we know that this is true because the only way to increment `len` is via `push(..)` which initializes value at `len - 1`.
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

    fn get_sink_res(&mut self) -> Result<(), StreamPoolSinkError> {
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

        let mut i = 0;
        while i < len {
            let index = (i + self.stream_index) % len;

            let (stream, addr) = (*self)
                .get_mut(index)
                .expect("stream should be initialized from 0..len");

            match stream.poll_next_unpin(cx) {
                Poll::Ready(Some(item)) => {
                    // if item is ready and returning early, poll again to poll other streams
                    cx.waker().wake_by_ref();
                    let addr = *addr;

                    if item.is_err() {
                        let _ = self.swap_remove(index);

                        // since stream at this index is removed, start at the same index next poll
                        self.stream_index = (i + self.stream_index) % len;
                    } else {
                        // start poll at the next index for next poll
                        self.stream_index = (i + 1 + self.stream_index) % len;
                    }

                    return Poll::Ready(Some((item.context(PollNextSnafu { addr }), addr)));
                }
                _ => {}
            }

            i += 1;
        }

        self.stream_index = (i + self.stream_index) % len;
        Poll::Pending
    }
}

impl<S: 'static + fmt::Debug + AsyncWrite + Unpin, const N: usize> StreamPool<S, N> {
    pub async fn send_to(
        &mut self,
        item: Bytes,
        addrs: &[SocketAddr],
    ) -> Result<(), StreamPoolSinkError> {
        self.send_filtered(item, |a| addrs.contains(a)).await
    }

    pub async fn send_filtered<Filter: Fn(&SocketAddr) -> bool>(
        &mut self,
        item: Bytes,
        filter: Filter,
    ) -> Result<(), StreamPoolSinkError> {
        poll_fn(|cx| {
            let len = self.len();

            if len == 0 && !self.sink_errors.is_empty() {
                return Poll::Ready(self.get_sink_res());
            }

            let indices = (0..len)
                .filter(|i| {
                    let (_stream, addr) = self.get(*i).expect("Element should exist with len");
                    filter(addr)
                })
                .collect::<Vec<_>>();

            // wait for poll ready and start_send
            for i in &indices {
                let (stream, addr) = self.get_mut(*i).expect("Element should exist with index");
                if let Err(e) =
                    ready!(stream.poll_ready_unpin(cx)).context(PollReadySnafu { addr: *addr })
                {
                    self.swap_remove(*i);
                    self.sink_errors.push(e);

                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
            }

            Poll::Ready(Ok(()))
        })
        .await?;

        let len = self.len();

        if len == 0 {
            return Ok(());
        }

        let indices = (0..len)
            .filter(|i| {
                let (_stream, addr) = self.get(*i).expect("Element should exist with len");
                filter(addr)
            })
            .collect::<Vec<_>>();

        // wait for poll ready and start_send
        for i in &indices {
            let (stream, addr) = self.get_mut(*i).expect("Element should exist with index");

            if let Err(e) = stream
                .start_send_unpin(item.clone())
                .context(StartSendSnafu { addr: *addr })
            {
                self.swap_remove(*i);
                self.sink_errors.push(e);
            }
        }

        poll_fn(|cx| {
            let len = self.len();

            if len == 0 && !self.sink_errors.is_empty() {
                return Poll::Ready(self.get_sink_res());
            }

            let indices = (0..len)
                .filter(|i| {
                    let (_stream, addr) = self.get(*i).expect("Element should exist with len");
                    filter(addr)
                })
                .collect::<Vec<_>>();

            for i in indices {
                let (stream, addr) = self.get_mut(i).expect("Element should exist with index");
                if let Err(e) =
                    ready!(stream.poll_flush_unpin(cx)).context(StartSendSnafu { addr: *addr })
                {
                    self.swap_remove(i);
                    self.sink_errors.push(e);

                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
            }

            Poll::Ready(Ok(()))
        })
        .await?;

        self.get_sink_res()
    }
}

impl<S: 'static + fmt::Debug + AsyncWrite + Unpin, const N: usize> Sink<Bytes>
    for StreamPool<S, N>
{
    type Error = StreamPoolSinkError;

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: Bytes) -> Result<(), Self::Error> {
        let mut len = self.len();

        if len == 0 {
            return Ok(());
        }

        let send = |this: &mut Self, n, t| {
            let (stream, addr) = this
                .get_mut(n)
                .expect("stream should be initialized from 0..len");

            stream
                .start_send_unpin(t)
                .context(StartSendSnafu { addr: *addr })
        };

        let mut i = 0;

        // skip the last element to avoid unnecessary cloning of item
        while i < len - 1 {
            if let Err(error) = send(&mut self, i, item.clone()) {
                let _removed = self
                    .swap_remove(i)
                    .expect("element to remove should exist since it was polled");

                self.sink_errors.push(error);

                len -= 1;
                continue;
            }

            i += 1;
        }

        // send item to last stream without cloning
        if let Err(error) = send(&mut self, i, item) {
            let _removed = self
                .pop()
                .expect("element to remove should exist since it was polled");

            self.sink_errors.push(error);
        }

        Ok(())
    }

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let mut i = 0;
        let mut len = self.len();

        while i < len {
            let (stream, addr) = (*self)
                .get_mut(i)
                .expect("stream should be initialized from 0..len");
            let poll_res =
                ready!(stream.poll_ready_unpin(cx)).context(PollReadySnafu { addr: *addr });

            if let Err(error) = poll_res {
                let _removed = self
                    .swap_remove(i)
                    .expect("element to remove should exist since it was polled");

                self.sink_errors.push(error);

                len -= 1;
                continue;
            }

            i += 1;
        }

        if len == 0 && !self.sink_errors.is_empty() {
            Poll::Ready(self.get_sink_res())
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let mut i = 0;
        let mut len = self.len();

        while i < len {
            let (stream, addr) = (*self)
                .get_mut(i)
                .expect("stream should be initialized from 0..len");
            let poll_res =
                ready!(stream.poll_flush_unpin(cx)).context(PollFlushSnafu { addr: *addr });

            if let Err(error) = poll_res {
                let _removed = self
                    .swap_remove(i)
                    .expect("element to remove should exist since it was polled");

                self.sink_errors.push(error);

                len -= 1;
                continue;
            }

            i += 1;
        }

        Poll::Ready(self.get_sink_res())
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let mut i = 0;
        let mut len = self.len();

        while i < len {
            let (stream, addr) = (*self)
                .get_mut(i)
                .expect("stream should be initialized from 0..len");
            let poll_res = ready!(stream.poll_close_unpin(cx));

            if let Err(error) = poll_res.context(PollCloseSnafu { addr: *addr }) {
                let _removed = self
                    .swap_remove(i)
                    .expect("element to remove should exist since it was polled");

                self.sink_errors.push(error);

                len -= 1;
                continue;
            }

            i += 1;
        }

        Poll::Ready(self.get_sink_res())
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
        #[snafu(display("[StreamPoolError] Failed poll_next for {addr}"))]
        PollNext {
            addr: SocketAddr,
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolError] Failed to split underlying stream"))]
        Split {
            source: SplitError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolError] Failed to unsplit underlying stream; read half and write half were not equal"))]
        RWUnequal,
        #[snafu(display("[StreamPoolError] Failed to unsplit underlying stream"))]
        Unsplit {
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

        pub fn as_io(&self) -> Option<&std::io::Error> {
            self.source.as_io()
        }

        pub fn errors(&self) -> impl Iterator<Item = &StreamPoolPollError> {
            std::iter::once(&self.source).chain(self.errors.iter())
        }

        pub fn io_errors(&self) -> impl Iterator<Item = &std::io::Error> {
            std::iter::once(&self.source)
                .chain(self.errors.iter())
                .map(|e| e.as_io())
                .filter_map(|e| e)
        }

        pub fn into_errors(self) -> impl Iterator<Item = StreamPoolPollError> {
            std::iter::once(self.source).chain(self.errors.into_iter())
        }
    }

    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum StreamPoolPollError {
        #[snafu(display("[StreamPoolPollError] Error in start_send to {addr}"))]
        StartSendError {
            addr: SocketAddr,
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolPollError] Error in poll_ready to {addr}"))]
        PollReadyError {
            addr: SocketAddr,
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolPollError] Error in poll_flush to {addr}"))]
        PollFlushError {
            addr: SocketAddr,
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
        #[snafu(display("[StreamPoolPollError] Error in poll_close to {addr}"))]
        PollCloseError {
            addr: SocketAddr,
            source: LengthDelimitedCodecError,
            backtrace: Backtrace,
        },
    }

    impl StreamPoolPollError {
        pub fn as_io(&self) -> Option<&std::io::Error> {
            match self {
                Self::StartSendError { source, .. } => source.as_io(),
                Self::PollReadyError { source, .. } => source.as_io(),
                Self::PollFlushError { source, .. } => source.as_io(),
                Self::PollCloseError { source, .. } => source.as_io(),
            }
        }
    }
}
