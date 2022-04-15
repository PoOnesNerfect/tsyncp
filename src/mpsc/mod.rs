use crate::codec::{length_delimited::LengthDelimitedCodec, DecodeMethod, EncodeMethod, Framed};
use bytes::Bytes;
use futures::Future;
use futures::Sink;
use snafu::Snafu;
use std::io;
use std::marker::PhantomData;
use std::task::Poll;
use tokio::net::{TcpStream, ToSocketAddrs};

#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::codec::JsonCodec>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T> = Sender<T, crate::codec::ProtobufCodec>;

type Result<T, E = Error> = std::result::Result<T, E>;

pub fn sender<A: ToSocketAddrs, T, F: EncodeMethod<T>>(
    dest: A,
) -> impl Future<Output = Result<Sender<T, F>>> {
    SenderBuilderFuture {
        dest,
        _phantom: PhantomData,
    }
}

/// Future returned by [sender] method in which awaiting it builds the [Sender].
///
/// This future can be optionally set custom configurations by calling methods on it such as [with_tls],
/// [with_codec], [with_frame_codec] before awaiting it.
pub struct SenderBuilderFuture<A: ToSocketAddrs, T, F: EncodeMethod<T>> {
    dest: A,
    _phantom: PhantomData<(T, F)>,
}

impl<A: ToSocketAddrs, T, F: EncodeMethod<T>> Future for SenderBuilderFuture<A, T, F> {
    type Output = Result<Sender<T, F>>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

pub struct Sender<T, F: EncodeMethod<T>, C = LengthDelimitedCodec> {
    framed: Framed<TcpStream, C>,
    _phantom: PhantomData<(T, F)>,
}

impl<T, F: EncodeMethod<T>> Sender<T, F> {
    pub fn with_tcp_stream(tcp_stream: TcpStream) -> Self {
        Self {
            framed: Framed::new(tcp_stream, LengthDelimitedCodec::new()),
            _phantom: PhantomData,
        }
    }

    pub fn into_tcp_stream(self) -> TcpStream {
        self.framed.into_inner()
    }

    pub fn into_framed(self) -> Framed<TcpStream> {
        self.framed
    }
}

impl<T, F: EncodeMethod<T>> Sink<T> for Sender<T, F> {
    type Error = Error;

    fn start_send(self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        todo!()
    }

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        todo!()
    }
}

// impl<T: Serialize + DeserializeOwned, F: Codec<T>> Stream for Sender<T, F> {
//     type Item = T;

//     fn poll_next(
//         self: std::pin::Pin<&mut Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> Poll<Option<Self::Item>> {
//         Poll::Pending
//     }
// }

/// Codec's error type
#[derive(Debug, Snafu)]
pub enum Error {
    /// Invalid length in frame header was received while decoding frame.
    #[snafu(display("Received invalid frame length {len} while decoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
    InvalidDecodingFrameLength {
        /// given invalid frame length
        len: usize,
        /// max frame length
        max_frame_length: usize,
    },
    /// Invalid length in frame header was received while encoding frame.
    #[snafu(display("Received invalid frame length {len} while encoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
    InvalidEncodingFrameLength {
        /// given invalid frame length
        len: usize,
        /// max frame length
        max_frame_length: usize,
    },
    /// returned from invalid inner IO Error
    #[snafu(display("Encountered IO Error while decoding frame: {source:?}"))]
    IoError {
        /// source IO Error
        source: io::Error,
    },
}

impl From<io::Error> for Error {
    fn from(src: io::Error) -> Self {
        Self::IoError { source: src }
    }
}
