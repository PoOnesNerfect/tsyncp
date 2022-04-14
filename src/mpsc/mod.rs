use crate::codec::{FrameDeserialize, FrameSerialize, Framed, LengthDelimitedCodec};
use bytes::Bytes;
use futures::Sink;
use snafu::Snafu;
use std::io;
use std::marker::PhantomData;
use std::task::Poll;
use tokio::net::{TcpStream, ToSocketAddrs};

#[cfg(feature = "json")]
pub type JsonSender<T> = Sender<T, crate::codec::JsonFrame>;

#[cfg(feature = "protobuf")]
pub type ProtobufSender<T> = Sender<T, crate::codec::ProtobufFrame>;

type Result<T, E = Error> = std::result::Result<T, E>;

pub async fn sender<T, F: FrameSerialize<T>>(dest: impl ToSocketAddrs) -> Result<Sender<T, F>> {
    todo!()
}

pub struct Sender<T, F: FrameSerialize<T>, C = LengthDelimitedCodec> {
    framed: Framed<TcpStream, C>,
    _phantom: PhantomData<(T, F)>,
}

impl<T, F: FrameSerialize<T>> Sender<T, F> {
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

impl<T, F: FrameSerialize<T>> Sink<T> for Sender<T, F> {
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
