use bytes::{Buf, BufMut, Bytes, BytesMut};
use errors::*;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder, Encoder, Framed};

/// default MAX_FRAME_LENGTH for codec
pub const MAX_FRAME_LENGTH: usize = 8 * 1024 * 1024;

type Result<T, E = CodecError> = std::result::Result<T, E>;

/// Length delimited bytes-to-frame encoding/decoding scheme that uses first n bits to represent header bytes' length. (1 <= n <= 3)
///
/// The implementation of the codec is similar to
/// [tokio_util]'s implementation of [length_delimited]; however, the differentiation is that tokio_util's implementation uses a fixed type (u32, unless specified otherwise using a builder) for frame length, whereas this crate's implementation uses varying types (u8 - u32) depending on the length of the frame.
///
/// # Limitations
///
/// - This codec allows limited customizations ([with_max_frame_length]), as it is a minimal implementation; if users would
/// like to customize things such as `length_adjustment`, `length_encoding_endianness`, etc.,
/// they should use [tokio_util::codec::length_delimited::VariedLengthDelimitedCodec] instead.
/// - This codec can encode and decode frames up to 2^29 bytes (536,870,912 bytes = 536GB), and if users
/// would like to send frames that are greater than 536GB, they should break it into multiple byte
/// objects before encoding with this codec.
///
/// # Getting started
///
/// # Implementation details
///
/// Length byte's first n bits are reserved to indicate whether or not there exists (n + 1)th length byte
/// (1 <= n <= 3).
///
/// ## Example 1
///
/// Say our frame payload is 100 bytes; it would be wasteful to use 2 or 3 bytes to
/// represent the number 100, when it could just be done in a single byte.
/// In this case, our frame will look like below:
///
/// ```text
/// +---- header bytes in binary ----+--------------------------------+
/// |           0 1100100            |          frame payload         |
/// +--------------------------------+--------------------------------+
/// ```
///
/// First bit `0` indicates that there is no subsequent byte to represent the frame length, meaning the frame header is a single byte. Subsequent 7 bits makes number value 100 in binary.
///
/// Since the first bit is used as a flag, a single header byte can only represent a value up to
/// 127; greater values would need an extra byte to represent it.
///
/// ## Example 2
///
/// In this example, let's use a frame with payload of 500 bytes. In this case, it will require at
/// least 2 bytes to represent the value:
///
/// ```text
/// +---- header bytes in binary ----+--------------------------------+
/// |       10 000001 11110100       |          frame payload         |
/// +--------------------------------+--------------------------------+
/// ```
///
/// Firt bit `1` indicates that there exists a second byte to represent the frame length, and the
/// second bit `0` indicates that there doesn't exist a third byte in the frame header, meaning that the header
/// frame is represented with 2 bytes. Subsequent 14 bits represent the number value 500.
///
/// Since the first 2 bits are used as flags, the 2 header bytes can only represent the byte length up to
/// 16,383. Values greater than this would need another byte.
///
/// ## Example 3
///
/// This time, say our frame payload is made of 2,000,000 bytes. In this case, 2 bytes (14 bits more
/// specifically) is not able to capture this value; in this case, it would need at least 3 bytes.
///
/// ```text
/// +------ header bytes in binary ------+--------------------------------+
/// |     110 11110 10000100 10000000    |          frame payload         |
/// +--------------------------+------------------------------------------+
/// ```
///
/// First two bits `11` indicate that there are at least 3 bytes in the frame header; subsequent bit `0`
/// indicates that there doesn't exist a 4th byte in the header bytes. And the subsequent 21
/// bits are used to represent the value 2,000,000. Since 3 bits are used as flags, 3 bytes will
/// only be able to represent values up to 2,097,151 bytes (~2MB).
///
/// But what if we wanted to send a frame that was 50GB?
///
/// ## Example 4
///
/// Finally, say we want to send a frame payload of length 50GB = 50,000,000 bytes. In this case, 3 bytes is
/// not even close to enough to represent this value. So, we need 4 bytes for it.
///
/// ```text
/// +---------- header bytes in binary -----------+--------------------------------+
/// |     111 00010 11111010 11110000 10000000    |          frame payload         |
/// +--------------------------+---------------------------------------------------+
/// ```
///
/// First three bits `111` indicate that there are 4 bytes used in the frame header; in this case,
/// we do not add another flag bit because 4 bytes (32 - 3 = 29 bits) are enough to represent values up to
/// 536,870,911 bytes (536GB), which should be more than enough for most cases.
///
/// [tokio_util]: https://docs.rs/tokio-util/latest/tokio_util/index.html
/// [length_delimited]: https://docs.rs/tokio-util/latest/tokio_util/codec/length_delimited/index.html
/// [tokio_util::codec::length_delimited::VariedLengthDelimitedCodec]: https://docs.rs/tokio-util/latest/tokio_util/codec/length_delimited/struct.VariedLengthDelimitedCodec.html
/// [with_max_frame_length]: VariedLengthDelimitedCodec::with_max_frame_length
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VariedLengthDelimitedCodec {
    max_frame_length: usize,
    state: DecodeState,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DecodeState {
    Head,
    Date(usize),
}

impl Default for VariedLengthDelimitedCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl VariedLengthDelimitedCodec {
    /// Creates a new `VariedLengthDelimitedCodec`.
    pub fn new() -> Self {
        Self {
            max_frame_length: MAX_FRAME_LENGTH,
            state: DecodeState::Head,
        }
    }

    /// Returns `Self` with custom `max_frame_length`
    ///
    /// Defaults to `MAX_FRAME_LENGTH = 8 * 1024 * 1024`
    pub fn with_max_frame_length(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            state: DecodeState::Head,
        }
    }

    /// Create a configured length delimited `Framed`
    ///
    /// # Example
    ///
    /// ```
    /// use tokio::io::{AsyncRead, AsyncWrite};
    /// use tsyncp::util::frame_codec::VariedLengthDelimitedCodec;
    ///
    /// fn write_frame<T: AsyncRead + AsyncWrite>(io: T) {
    ///     let framed = VariedLengthDelimitedCodec::new().into_framed(io);
    /// }
    /// ```
    pub fn into_framed<T>(self, inner: T) -> Framed<T, VariedLengthDelimitedCodec>
    where
        T: AsyncRead + AsyncWrite,
    {
        Framed::new(inner, self)
    }

    fn decode_head(&mut self, src: &mut BytesMut) -> Result<Option<usize>> {
        if src.len() == 0 {
            return Ok(None);
        }

        // get length of header bytes
        let header_len = match src[0] & 0b111_00000 {
            0b111_00000 => 4,
            0b110_00000 => 3,
            0b100_00000 | 0b101_00000 => 2,
            _ => 1,
        };

        // if there aren't enough bytes to fill header, then return
        if src.len() < header_len {
            return Ok(None);
        }

        let header_mask = match header_len {
            1 => 0b0_1111111,
            2 => 0b00_111111,
            3 | 4 => 0b000_11111,
            _ => unreachable!("value is set as 1 - 4 above"),
        };
        src[0] &= header_mask;

        // get payload length from header info
        let payload_len = src.get_uint(header_len) as usize;

        if payload_len > self.max_frame_length {
            return InvalidDecodingFrameLengthSnafu {
                len: payload_len,
                max_frame_length: self.max_frame_length,
            }
            .fail();
        }

        // Ensure that the buffer has enough space to read the incoming payload
        src.reserve(payload_len);

        Ok(Some(payload_len))
    }

    fn decode_data(&self, n: usize, src: &mut BytesMut) -> Option<BytesMut> {
        // At this point, the buffer has already had the required capacity
        // reserved. All there is to do is read.
        if src.len() < n {
            return None;
        }

        Some(src.split_to(n))
    }
}

impl Decoder for VariedLengthDelimitedCodec {
    type Item = BytesMut;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        let payload_len = match self.state {
            DecodeState::Head => match self.decode_head(src)? {
                Some(payload_len) => {
                    self.state = DecodeState::Date(payload_len);
                    payload_len
                }
                None => return Ok(None),
            },
            DecodeState::Date(payload_len) => payload_len,
        };

        match self.decode_data(payload_len, src) {
            Some(data) => {
                // Update the decode state
                self.state = DecodeState::Head;

                // Make sure the buffer has enough space to read the next head; at least 4 bytes.
                src.reserve(4);

                Ok(Some(data))
            }
            None => Ok(None),
        }
    }
}

impl Encoder<Bytes> for VariedLengthDelimitedCodec {
    type Error = CodecError;

    fn encode(&mut self, data: Bytes, dst: &mut BytesMut) -> Result<()> {
        let payload_len = data.len();

        if payload_len > self.max_frame_length {
            return InvalidEncodingFrameLengthSnafu {
                len: payload_len,
                max_frame_length: self.max_frame_length,
            }
            .fail();
        }

        // single byte max length
        let (header_frame, header_len) = if payload_len < 1 << (8 - 1) {
            (payload_len, 1)
        } else if payload_len < 1 << (8 * 2 - 2) {
            (payload_len | (0b1 << 15), 2)
        } else if payload_len < 1 << (8 * 3 - 3) {
            (payload_len | (0b11 << 22), 3)
        } else {
            (payload_len | (0b111 << 29), 4)
        };

        // Reserve capacity in the destination buffer to fit the frame and
        // length field.
        dst.reserve(payload_len + header_len);

        dst.put_uint(header_frame as u64, header_len);
        dst.extend_from_slice(&data);

        Ok(())
    }
}

pub mod errors {
    use snafu::{Backtrace, GenerateImplicitData, Snafu};
    use std::io;

    /// Codec's error type
    #[derive(Debug, Snafu)]
    #[snafu(visibility(pub(super)))]
    pub enum CodecError {
        /// Invalid length in frame header was received while decoding frame.
        #[snafu(display("Received invalid frame length {len} while decoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
        InvalidDecodingFrameLength {
            /// given invalid frame length
            len: usize,
            /// max frame length
            max_frame_length: usize,
            backtrace: Backtrace,
        },
        /// Invalid length in frame header was received while encoding frame.
        #[snafu(display("Received invalid frame length {len} while encoding; bytes' length must be greater 0 and less than {max_frame_length}"))]
        InvalidEncodingFrameLength {
            /// given invalid frame length
            len: usize,
            /// max frame length
            max_frame_length: usize,
            backtrace: Backtrace,
        },
        /// Returned from invalid inner IO Error.
        #[snafu(display("Encountered IO Error while decoding frame"))]
        IoError {
            /// Source IO Error
            source: io::Error,
            backtrace: Backtrace,
        },
    }

    impl From<io::Error> for CodecError {
        fn from(src: io::Error) -> Self {
            Self::IoError {
                source: src,
                backtrace: Backtrace::generate(),
            }
        }
    }

    impl CodecError {
        /// Try to view the error as [std::io::Error].
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn as_io(&self) -> Option<&io::Error> {
            if let Self::IoError { source, .. } = self {
                return Some(source);
            }

            None
        }

        /// Try converting the error as [std::io::Error].
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn into_io(self) -> Option<io::Error> {
            if let Self::IoError { source, .. } = self {
                return Some(source);
            }

            None
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

        /// Check if the error is from connection reset.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_connection_reset(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == io::ErrorKind::ConnectionReset)
                .unwrap_or_default()
        }

        /// Check if the error is from connection refused.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_connection_refused(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == io::ErrorKind::ConnectionRefused)
                .unwrap_or_default()
        }

        /// Check if the error is from connection aborted.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_connection_aborted(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == io::ErrorKind::ConnectionAborted)
                .unwrap_or_default()
        }

        /// Check if the error is from not connected.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_not_connected(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == io::ErrorKind::NotConnected)
                .unwrap_or_default()
        }

        /// Check if the error is from broken pipe.
        ///
        /// This is useful to see if the returned error is from the underlying TCP connection.
        /// This method will be bubbled up with the error, and also be available at the highest
        /// level.
        pub fn is_broken_pipe(&self) -> bool {
            self.as_io()
                .map(|e| e.kind() == io::ErrorKind::BrokenPipe)
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::{
        faker::lorem::en::{Sentence, Sentences},
        Dummy, Fake, Faker,
    };
    use serde::{Deserialize, Serialize};
    use std::iter;

    #[derive(Debug, Serialize, Deserialize, Dummy, PartialEq)]
    struct ExampleStruct {
        #[dummy(faker = "1..2000")]
        example_field1: usize,
        #[dummy(faker = "1000..2000")]
        example_field2: String,
        example_field3: (String, Vec<String>),
    }

    #[test]
    fn test_codec() -> Result<()> {
        let mut codec = VariedLengthDelimitedCodec::new();
        let mut buffer = BytesMut::new();

        let faked1: ExampleStruct = Faker.fake();

        let ser1: Bytes = serde_json::to_vec(&faked1).unwrap().into();
        codec.encode(ser1, &mut buffer)?;

        let faked2: ExampleStruct = Faker.fake();
        let ser2: Bytes = serde_json::to_vec(&faked2).unwrap().into();
        codec.encode(ser2, &mut buffer)?;

        let faked3: String = Sentence(100000..105000).fake();
        codec.encode(faked3.clone().into_bytes().into(), &mut buffer)?;

        let faked4: Vec<String> = Sentences(10000..10500).fake();
        codec.encode(serde_json::to_vec(&faked4).unwrap().into(), &mut buffer)?;

        let decoded = codec.decode(&mut buffer)?.unwrap();
        let deser: ExampleStruct = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(deser, faked1);

        let decoded = codec.decode(&mut buffer)?.unwrap();
        let deser: ExampleStruct = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(deser, faked2);

        let decoded = codec.decode(&mut buffer)?.unwrap();
        assert_eq!(decoded, faked3);

        let decoded = codec.decode(&mut buffer)?.unwrap();
        let deser: Vec<String> = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(deser, faked4);

        Ok(())
    }

    fn test_encoded_bytes(len: usize, expected_header_len: usize) -> Result<()> {
        let mut codec = VariedLengthDelimitedCodec::new();
        let mut buffer = BytesMut::new();

        let bytes = iter::repeat(b'c').take(len).collect::<Bytes>();
        codec.encode(bytes.into(), &mut buffer)?;

        // check that encoded bytes' length is equal to expected header_len + len
        assert_eq!(buffer.len(), expected_header_len + len);

        // check that decoded bytes' length is equal to original bytes len
        let decoded = codec.decode(&mut buffer)?.unwrap();
        assert_eq!(decoded.len(), len);

        Ok(())
    }

    #[test]
    fn test_edge_cases() -> Result<()> {
        // 0 bytes should have 1 byte header + 0 bytes payload
        test_encoded_bytes(0, 1)?;

        // 1 byte should have 1 byte header + 1 byte payload
        test_encoded_bytes(1, 1)?;

        // 127 bytes should have 1 byte header + 127 bytes payload
        test_encoded_bytes(127, 1)?;

        // 128 bytes should have 2 byte header + 128 bytes payload
        test_encoded_bytes(128, 2)?;

        // 16383 bytes should have 2 byte header + 16383 bytes payload
        test_encoded_bytes(16383, 2)?;

        // 16384 bytes should have 3 byte header + 16384 bytes payload
        test_encoded_bytes(16384, 3)?;

        // 2097151 bytes should have 3 byte header + 2097151 bytes payload
        test_encoded_bytes(2097151, 3)?;

        // 2097152 bytes should have 4 byte header + 2097152 bytes payload
        test_encoded_bytes(2097152, 4)?;

        Ok(())
    }
}
