//! Contains [EncodeMethod] and [DecodeMethod] to generically implement encoding and decoding over
//! different libraries.
//!
//! Module also contains already implemented codecs to be easily used.

use bytes::{Bytes, BytesMut};
use std::io;

#[cfg(feature = "serde")]
use serde::{de::DeserializeOwned, Serialize};

/// Trait for encoding a given item into bytes.
///
/// Generic parameter `T` is the data that will be encoded.
/// When implementing this trait, you can specify trait requirements for `T`, so that,
/// any types that implements that trait will be able to be encoded.
///
/// # Example
///
/// Below example will serialize any item that implements `serde::Serialize` into json.
///
/// ```no_run
/// use tsyncp::util::codec::EncodeMethod;
/// use bytes::Bytes;
///
/// pub struct MyCustomCodec;
///
/// impl<T: serde::Serialize> EncodeMethod<T> for MyCustomCodec {
///     type Error = serde_json::Error;
///
///     fn encode(data: &T) -> Result<Bytes, Self::Error> {
///         serde_json::to_vec(data).map(Into::into)
///     }
/// }
/// ```
pub trait EncodeMethod<T> {
    /// Error returned by associated method `encode(_)`.
    type Error: 'static + std::error::Error;

    /// Encode given data.
    fn encode(data: &T) -> Result<Bytes, Self::Error>;
}

/// Trait for decoding given bytes into an item.
///
/// Generic parameter `T` is the data that will be decoded.
/// When implementing this trait, you can specify trait requirements for `T`, so that,
/// any types that implements that trait will be able to be decoded.
///
/// # Example
///
/// Below example will deserialize any item that implements `serde::de::DeserializeOwned` from json bytes.
///
/// ```no_run
/// use tsyncp::util::codec::DecodeMethod;
/// use bytes::BytesMut;
///
/// pub struct MyCustomCodec;
///
/// impl<T: serde::de::DeserializeOwned> DecodeMethod<T> for MyCustomCodec {
///     type Error = serde_json::Error;
///
///     fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
///         serde_json::from_slice(bytes.as_ref())
///     }
/// }
/// ```
pub trait DecodeMethod<T> {
    /// Error returned by associated method `decode(_)`.
    type Error: 'static + std::error::Error;

    /// Decode given data.
    fn decode(bytes: BytesMut) -> Result<T, Self::Error>;
}

/// Unit struct that encodes and decodes empty tuple. Used for sending empty payloads.
///
/// `EmptyCodec` only implements traits for empty tuple `()`, and is created mainly for [barrier](crate::barrier) primitives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EmptyCodec;

impl EncodeMethod<()> for EmptyCodec {
    type Error = io::Error;

    fn encode(_: &()) -> Result<Bytes, Self::Error> {
        Ok(Bytes::from([0].as_ref()))
    }
}

impl DecodeMethod<()> for EmptyCodec {
    type Error = io::Error;

    fn decode(bytes: BytesMut) -> Result<(), Self::Error> {
        if bytes.len() == 1 {
            if bytes[0] == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "[EmptyCodec] Received byte that is not 0u8",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "[EmptyCodec] Received bytes lenngth greater than 1",
            ))
        }
    }
}

#[cfg(feature = "json")]
pub use json::*;
#[cfg(feature = "json")]
mod json {
    use super::*;

    /// Unit struct that encodes and decodes data as json objects.
    ///
    /// This implements serializing/deserializing as json using `serde` and `serde_json`.
    /// Think of it as a light wrapper around `serde_json`.
    ///
    /// You can use this codec in any of the primitives for any data structs that implements
    /// `serde::Serialize` and `serde::Deserialize`.
    ///
    /// All primitives already introduces type alias for this codec so you can just use that for simplicity:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    ///
    /// pub type JsonSender<T> = mpsc::Sender<T, tsyncp::util::codec::JsonCodec>;
    ///
    /// pub type JsonReceiver<T, const N: usize = 0> = mpsc::Receiver<T, tsyncp::util::codec::JsonCodec, N>;
    /// ```
    ///
    /// ### Example
    /// ```no_run
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::mpsc;
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy {
    ///     field1: String,
    ///     field2: u64,
    ///     field3: Vec<u8>,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
    ///     let tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:8000").await?;
    ///     Ok(())
    /// }
    /// ```
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct JsonCodec;

    impl<T: Serialize> EncodeMethod<T> for JsonCodec {
        type Error = serde_json::Error;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            serde_json::to_vec(data).map(Into::into)
        }
    }

    impl<T: DeserializeOwned> DecodeMethod<T> for JsonCodec {
        type Error = serde_json::Error;

        fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
            serde_json::from_slice(bytes.as_ref())
        }
    }
}

#[cfg(feature = "bincode")]
pub use bincode_mod::*;
#[cfg(feature = "bincode")]
mod bincode_mod {
    use super::*;

    /// Unit struct that encodes and decodes data using bincode.
    ///
    /// This implements encoding/decoding `bincode` crate.
    /// Think of it as a light wrapper around `bincode`.
    ///
    /// You can use this codec in any of the primitives for any data structs that implements
    /// `serde::Serialize` and `serde::Deserialize`.
    ///
    /// All primitives already introduces type alias for this codec so you can just use that for simplicity:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    ///
    /// pub type BincodeSender<T> = mpsc::Sender<T, tsyncp::util::codec::BincodeCodec>;
    ///
    /// pub type BincodeReceiver<T, const N: usize = 0> = mpsc::Receiver<T, tsyncp::util::codec::BincodeCodec, N>;
    /// ```
    ///
    /// ### Example
    /// ```no_run
    /// use serde::{Serialize, Deserialize};
    /// use tsyncp::mpsc;
    ///
    /// #[derive(Debug, Serialize, Deserialize)]
    /// struct Dummy {
    ///     field1: String,
    ///     field2: u64,
    ///     field3: Vec<u8>,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let rx: mpsc::BincodeReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
    ///     let tx: mpsc::BincodeSender<Dummy> = mpsc::sender_to("localhost:8000").await?;
    ///     Ok(())
    /// }
    /// ```
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct BincodeCodec;

    impl<T: Serialize> EncodeMethod<T> for BincodeCodec {
        type Error = bincode::Error;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            bincode::serialize(data).map(Into::into)
        }
    }

    impl<T: DeserializeOwned> DecodeMethod<T> for BincodeCodec {
        type Error = bincode::Error;

        fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
            bincode::deserialize(bytes.as_ref())
        }
    }
}

#[cfg(feature = "prost")]
pub use prost_mod::*;
#[cfg(feature = "prost")]
mod prost_mod {
    use super::*;

    /// Unit struct that encodes and decodes data as protobuf objects.
    ///
    /// This implements serializing/deserializing as protobuf using `prost`.
    /// Think of it as a light wrapper around `prost`.
    ///
    /// You can use this codec in any of the primitives for any data structs that implements
    /// `prost::Message`.
    ///
    /// All primitives already introduces type alias for this codec so you can just use that for simplicity:
    ///
    /// ```no_run
    /// use tsyncp::mpsc;
    ///
    /// pub type ProstSender<T> = mpsc::Sender<T, tsyncp::util::codec::ProstCodec>;
    ///
    /// pub type ProstReceiver<T, const N: usize = 0> = mpsc::Receiver<T, tsyncp::util::codec::ProstCodec, N>;
    /// ```
    ///
    /// ### Example
    /// ```no_run
    /// use prost::Message;
    /// use tsyncp::mpsc;
    ///
    /// #[derive(Message)]
    /// struct Dummy {
    ///     #[prost(string, tag = "1")]
    ///     field1: String,
    ///     #[prost(uint64, tag = "2")]
    ///     field2: u64,
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() -> color_eyre::Result<()> {
    ///     let rx: mpsc::ProstReceiver<Dummy> = mpsc::receiver_on("localhost:8000").await?;
    ///     let tx: mpsc::ProstSender<Dummy> = mpsc::sender_to("localhost:8000").await?;
    ///     Ok(())
    /// }
    /// ```
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ProstCodec;

    impl<T: prost::Message> EncodeMethod<T> for ProstCodec {
        type Error = prost::EncodeError;

        fn encode(data: &T) -> Result<Bytes, Self::Error> {
            Ok(data.encode_to_vec().into())
        }
    }

    impl<T: prost::Message + Default> DecodeMethod<T> for ProstCodec {
        type Error = prost::DecodeError;

        fn decode(mut bytes: BytesMut) -> Result<T, Self::Error> {
            T::decode(&mut bytes)
        }
    }
}

// #[cfg(feature = "rkyv")]
// pub use rkyv_mod::*;
// #[cfg(feature = "rkyv")]
// mod rkyv_mod {
//     use super::{DecodeMethod, EncodeMethod};
//     use bytecheck::CheckBytes;
//     use bytes::{Bytes, BytesMut};
//     use rkyv::{
//         ser::serializers::{
//             AlignedSerializer, AllocScratch, AllocScratchError, CompositeSerializer,
//             CompositeSerializerError, FallbackScratch, HeapScratch, SharedSerializeMap,
//             SharedSerializeMapError,
//         },
//         AlignedVec,
//     };

//     #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
//     pub struct RkyvCodec;

//     impl<
//             T: rkyv::Serialize<
//                 CompositeSerializer<
//                     AlignedSerializer<AlignedVec>,
//                     FallbackScratch<HeapScratch<256_usize>, AllocScratch>,
//                     SharedSerializeMap,
//                 >,
//             >,
//         > EncodeMethod<T> for RkyvCodec
//     {
//         // type Error = CompositeSerializerError<Infallible, AllocScratchError>;
//         type Error = CompositeSerializerError<
//             std::convert::Infallible,
//             AllocScratchError,
//             SharedSerializeMapError,
//         >;

//         fn encode(data: &T) -> Result<Bytes, Self::Error> {
//             rkyv::to_bytes::<_, 256>(data).map(|bytes| Bytes::from(bytes.to_vec()))
//         }
//     }

//     impl<T: rkyv::Archive + CheckBytes> DecodeMethod<T> for RkyvCodec {
//         type Error = std::convert::Infallible;

//         fn decode(bytes: BytesMut) -> Result<T, Self::Error> {
//             let archived = rkyv::check_archived_root::<T>(&bytes[..]).unwrap();
//             let t: T = archived.deserialize(&mut rkyv::Infallible).unwrap();

//             // Ok(t)
//             todo!();
//         }
//     }
// }
