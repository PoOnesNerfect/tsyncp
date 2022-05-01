use color_eyre::{Report, Result};
use env_logger::Env;
use log::{error, info};
use prost::Message;
use std::time::{Duration, Instant};
use tsyncp::{broadcast, channel, mpsc};

const COUNT: u64 = 100_000;
const ADDR: &str = "localhost:8001";

#[derive(Clone, Message)]
struct Dummy {
    #[prost(string, tag = "1")]
    field1: String,
    #[prost(uint64, tag = "2")]
    field2: u64,
    #[prost(uint64, tag = "3")]
    field3: u64,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    if let Err(error) = try_main().await {
        error!("Error encountered while running service: \n\n{:?}", error);
        // error.downcast_ref::<Box<dyn snafu::ErrorCompat + Send + Sync + std::fmt::Debug>>();
    }
}

async fn try_main() -> Result<()> {
    let handle1 = tokio::spawn(async move {
        let mut ch: channel::ProtobufChannel<Dummy> = channel::channel_on(ADDR)
            .filter(|a| {
                info!("connected to {a}");
                true
            })
            .await?;

        // send data to channel
        let now = Instant::now();
        for i in 0..COUNT {
            ch.send(Dummy {
                field1: "hello world".to_string(),
                field2: 123213,
                field3: i,
            })
            .await?;
        }
        let duration = Instant::now() - now;
        info!("sending {} msgs took {:?}", COUNT, duration);

        let mut i = 0;

        // receive data from channel
        let now = Instant::now();
        while let Some(Ok(_)) = ch.recv().await {
            i += 1;

            if i % 10_000 == 0 {
                // info!("{n}: received {i} msgs");
                if i % COUNT == 0 {
                    break;
                }
            }
        }
        let duration = Instant::now() - now;

        info!("receiving {COUNT} msgs took {duration:?}");

        // split and send/receive data concurrently.
        let (rx, tx) = ch.split();

        // below two lines are just for showing what types they are.
        let mut rx: broadcast::ProtobufReceiver<Dummy> = rx;
        let mut tx: mpsc::ProtobufSender<Dummy> = tx;

        let tx_handle = tokio::spawn(async move {
            let now = Instant::now();
            for i in 0..COUNT {
                tx.send(Dummy {
                    field1: "hello world".to_string(),
                    field2: 123213,
                    field3: i,
                })
                .await?;
            }
            let duration = Instant::now() - now;

            info!("sending concurrently {} msgs took {:?}", COUNT, duration);
            Ok::<_, Report>(())
        });

        let rx_handle = tokio::spawn(async move {
            let mut i = 0;

            let now = Instant::now();
            while let Some(Ok(_)) = rx.recv().await {
                i += 1;

                if i % 10_000 == 0 {
                    // info!("{n}: received {i} msgs");
                    if i % COUNT == 0 {
                        break;
                    }
                }
            }
            let duration = Instant::now() - now;

            info!("receiving concurrently {COUNT} msgs took {duration:?}");
        });

        tx_handle.await??;
        rx_handle.await?;

        Ok::<_, color_eyre::Report>(())
    });

    let handle2 = tokio::spawn(async move {
        let mut ch: channel::ProtobufChannel<Dummy> = channel::channel_to(ADDR)
            .retry(Duration::from_millis(500), 100)
            .await?;

        let mut i = 0;

        // receive data from channel.
        let now = Instant::now();
        while let Some(Ok(_)) = ch.recv().await {
            i += 1;

            if i % 10_000 == 0 {
                // info!("{n}: received {i} msgs");
                if i % COUNT == 0 {
                    break;
                }
            }
        }
        let duration = Instant::now() - now;

        info!("receiving {COUNT} msgs took {duration:?}");

        // send data to channel.
        let now = Instant::now();
        for i in 0..COUNT {
            ch.send(Dummy {
                field1: "hello world".to_string(),
                field2: 123213,
                field3: i,
            })
            .await?;
        }
        let duration = Instant::now() - now;
        info!("sending {} msgs took {:?}", COUNT, duration);

        // split and send/receive data concurrently.
        let (rx, tx) = ch.split();

        // below two lines are just for showing what types they are.
        let mut rx: broadcast::ProtobufReceiver<Dummy> = rx;
        let mut tx: mpsc::ProtobufSender<Dummy> = tx;

        let tx_handle = tokio::spawn(async move {
            let now = Instant::now();
            for i in 0..COUNT {
                tx.send(Dummy {
                    field1: "hello world".to_string(),
                    field2: 123213,
                    field3: i,
                })
                .await?;
            }
            let duration = Instant::now() - now;

            info!("sending concurrently {} msgs took {:?}", COUNT, duration);
            Ok::<_, Report>(())
        });

        let rx_handle = tokio::spawn(async move {
            let mut i = 0;

            let now = Instant::now();
            while let Some(Ok(_)) = rx.recv().await {
                i += 1;

                if i % 10_000 == 0 {
                    // info!("{n}: received {i} msgs");
                    if i % COUNT == 0 {
                        break;
                    }
                }
            }
            let duration = Instant::now() - now;

            info!("receiving concurrently {COUNT} msgs took {duration:?}");
        });

        tx_handle.await??;
        rx_handle.await?;

        Ok::<_, color_eyre::Report>(())
    });

    handle1.await??;
    handle2.await??;

    Ok(())
}
