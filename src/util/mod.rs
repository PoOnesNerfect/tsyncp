//! Contains utility traits and structs that help implement primitives.
//!
//! This module includes traits that generalizes essential parts of the primitives,
//! such as [Accept], [Split], and [codec::EncodeMethod] and [codec::DecodeMethod];
//! and implemeneters of these traits such as [listener::ReadListener] and [listener::WriteListener],
//! and [codec::JsonCodec] and [codec::ProstCodec].
//!
//! These implementations make all primitives very generic and allows room for flexible future development.

pub mod codec;
pub mod frame_codec;
pub mod listener;
pub mod stream_pool;
pub mod tcp;

mod split;
pub use split::Split;

mod accept;
pub use accept::Accept;

/// Type alias for [Framed] with our custom length delimited codec.
///
/// [Framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
pub type Framed<T, U = crate::util::frame_codec::VariedLengthDelimitedCodec> =
    tokio_util::codec::Framed<T, U>;

/// Settings used for TCP streams as `nodelay` and `ttl`.
///
/// This struct is used as the associated type `Config` for the trait [Accept] by `TcpListener` .
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TcpStreamSettings {
    /// TCP setting `nodelay`.
    pub nodelay: Option<bool>,
    /// TCP setting `ttl`.
    pub ttl: Option<u32>,
}
