pub mod accept;
pub mod codec;
pub mod frame_codec;
pub mod listener;
pub mod split;
pub mod stream_pool;
pub mod tcp;

/// Type alias for [Framed] with our custom length delimited codec.
///
/// [Framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
pub type Framed<T, U = crate::util::frame_codec::VariedLengthDelimitedCodec> =
    tokio_util::codec::Framed<T, U>;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TcpStreamSettings {
    pub nodelay: Option<bool>,
    pub ttl: Option<u32>,
}
