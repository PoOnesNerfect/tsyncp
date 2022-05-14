# tsyncp

[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]

[crates-badge]: https://img.shields.io/crates/v/tsyncp.svg
[crates-url]: https://crates.io/crates/tsyncp
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/PoOnesNerfect/tsyncp/blob/main/LICENSE

### Synchronization primitives over TCP for message passing.

Major rust libraries such as [std] and [tokio] provide great synchronization primitives for
message-passing between threads and tasks. However, there are not many libraries
that provide similar APIs that can be used over the network.

**Tsyncp** tries to fill the gap by providing the similar APIs (mpsc, broadcast, barrier, etc) over TCP. If you
have a personal project where it only has a few services running, and they need to pass some data to each other;
instead of setting up a message-broker service, you can use tsyncp to easily pass data between them.

**Tsyncp** also allows customizing different Serialization/Deserialization methods to encode/decode
data; currently, supported schemes straight from the library are _Json_, _Protobuf_, and _Bincode_; however,
users can very easily implement their own [EncodeMethod] and [DecodeMethod].

## Provided APIs

Currently, **tsyncp** provides 5 types of channels:

-   **mpsc**: Multi-producer/single-consumer channel.
-   **broadcast**: Sigle-producer/multi-consumer channel.
-   **barrier**: Ensures multiple waiters wait for the barrier to release.
-   **channel**: Generic single-connection channel for sending/receiving data.
    Can split into _Sender_ and _Receiver_ pair.
-   **multi_channel**: Generic multi-connection channel for sending/receiving data.
    Can split into _Sender_ and _Receiver_ pair.

## Example

Goal of the project is to provide simple, intuitive but extendable primitives to pass data over the network.
That's why this library uses Future-chaining extensively.

Getting started is as easy as:

## mpsc::Receiver

```rust
use color_eyre::Result;
use serde::{Serialize, Deserialize};
use tsyncp::mpsc;

#[derive(Debug, Serialize, Deserialize)]
struct Dummy {
    field1: String,
    field2: u64,
    field3: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114").await?;

    // accept a new connection coming from a sender application.
    rx.accept().await?;

    // after accepting connection, you can start receiving data from the receiver.
    if let Some(Ok(item)) = rx.recv().await {
        // below line is to show the type of received item.
        let item: Dummy = item;

        println!("received item: {item:?}");
    }

    Ok(())
}
```

But you can easily extend it by chaining futures as below:

```rust
use color_eyre::{Result, Report};
use serde::{Serialize, Deserialize};
use tsyncp::mpsc;

#[derive(Debug, Serialize, Deserialize)]
struct Dummy {
    field1: String,
   field2: u64,
   field3: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut rx: mpsc::JsonReceiver<Dummy> = mpsc::receiver_on("localhost:11114")
        .limit(10)                              // limit allowed connections to 10.
        .set_tcp_reuseaddr(true)                // set tcp config reuseaddr to `true`.
        .accept()                               // accept connection. (default: 1)
        .to_limit()                             // accept until limit is reached. (10)
        .handle(|a| println!("{a} connected!")) // print address when a connection is accepted.
        .await?;

    // At this point, the receiver has 10 connections in the connection pool,
    // which all have `reuseaddr` as `true`.
    while let Some(Ok((item, addr))) = rx.recv().with_addr().await {
        println!("received item: {item:?} from {addr}");
    }

    Ok(())
 }
```

I just vomited a whole bunch of chains, but you can just use any chains that fits your neck.

## mpsc::Sender

```rust
use color_eyre::{Result, Report};
use serde::{Serialize, Deserialize};
use tsyncp::mpsc;

#[derive(Debug, Serialize, Deserialize)]
struct Dummy {
    field1: String,
    field2: u64,
    field3: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:11114").await?;

    let dummy = Dummy {
        field1: String::from("hello world"),
        field2: 1234567,
        field3: vec![1, 2, 3, 4]
    };

    tx.send(dummy).await?;

    Ok(())
}
```

But you can also extend it by chaining the futures as:

```rust
use color_eyre::{Result, Report};
use serde::{Serialize, Deserialize};
use tsyncp::mpsc;

#[derive(Debug, Serialize, Deserialize)]
struct Dummy {
    field1: String,
    field2: u64,
    field3: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    use std::time::Duration;

    let retry_interval = Duration::from_millis(500);
    let max_retries = 100;

    let mut tx: mpsc::JsonSender<Dummy> = mpsc::sender_to("localhost:11114")
        .retry(retry_interval, max_retries) // retry connecting 100 times every 500 ms.
        .set_tcp_reuseaddr(true)            // set tcp config reuseaddr to `true`.
        .await?;

    let dummy = Dummy {
        field1: String::from("hello world"),
        field2: 1234567,
        field3: vec![1, 2, 3, 4]
    };

    // send some item.
    tx.send(dummy).await?;

    Ok(())
}
```

The [API documentation](https://docs.rs/tsyncp/) has a very detailed guide on how to use the primitives. So please check them out!

## Future Plans

If enough people find this library useful or interesting, I will work primarily on the following:

-   **Unit testing**: This library currently lacks extensive unit testing, due to the author's limited time resources.
-   **Benchmarking**: Currently, benchmarking was done locally and with simple messages.
-   **CI/CD Flow**: Currently, I've only pushed manually to git and to crates.io.
-   **Encrypted Streams**: _Tls_ and _Noise_. Technically, this library is already generic enough that it can have any type of bytestreams underneath.
    So, it should not be that rough implementing options for Tls and Noise streams! (fingers crossed...!)
-   **More primitives**: Oneshot, pubsub, and client/server primitives?
-   **Supporting other Encoding/Decoding implementations**: Right now, I'm thinking libraries like _rkyv_ and _speedy_;
    if anyone has any other options I can look into, please let me know!

**_Note_**: Tsyncp is built on [tokio]; and thus may not be compatible with other async runtimes like async-std.

**_Note_**: If you're worried about the quality of implementation and skills of the author, you shouldn't worry! (too much...)
This library is a relatively thin layer on top of [tokio::net::TcpStream] for TCP,
[tokio_util::codec::Framed] for byte stream framing, and [serde], [serde_json], [Prost] and [bincode]. for serializing byte data!
You can think of this library as a salad bowl made from the best ingredients in town and a light touch of amateur homemade sauces.

That being said, I did add a couple of my own sauces into it. The two flavours that I added are:

-   Very simple implementation of [VariedLengthDelimitedCodec](https://docs.rs/tsyncp/latest/tsyncp/util/frame_codec/struct.VariedLengthDelimitedCodec.html),
    which encodes/decodes varying header length depending on the size of the byte data.
-   Minimal implementation of TCP stream pool [StreamPool](https://docs.rs/tsyncp/latest/tsyncp/util/stream_pool/struct.StreamPool.html),
    which allows pushing and popping streams from the pool, streaming data from the pool, and broadcasting data into the pool.

You can take a look at the implementations of these, which both are pretty simple.
And I tried my best not to put too much of my personal opinions into it!

But still, since this library is still a baby, if you find any concerns in the code, please contact me at **jack.y.l.dev@gmail.com**!

_Warning: Tsyncp is not a message-broker nor it tries to be;
it's just a message-passing library for simple and convenient use cases._

_Warning: Tsyncp is still WIP! It's very usable, but still needs some encode/decode features implemented, extensive testing, documentations, and examples._

[std]: https://doc.rust-lang.org/stable/std/
[tokio]: https://docs.rs/tokio/latest/tokio/index.html
[encodemethod]: https://docs.rs/tsyncp/latest/tsyncp/util/codec/trait.EncodeMethod.html
[decodemethod]: https://docs.rs/tsyncp/latest/tsyncp/util/codec/trait.DecodeMethod.html
[tokio::net::tcpstream]: https://docs.rs/tokio/latest/tokio/net/struct.TcpStream.html
[tokio_util::codec::framed]: https://docs.rs/tokio-util/latest/tokio_util/codec/struct.Framed.html
[serde]: https://crates.io/crates/serde
[serde_json]: https://crates.io/crates/serde_json
[prost]: https://crates.io/crates/prost
