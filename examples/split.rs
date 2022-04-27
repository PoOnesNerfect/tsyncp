use color_eyre::{Report, Result};
use env_logger::Env;
use futures::future::try_join_all;
use log::{error, info};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tsyncp::{broadcast, channel, mpsc, multi_channel};

const COUNT: usize = 1000_000;
const LEN: usize = 10;

const ADDR: &str = "localhost:8000";

#[derive(
    Clone, Serialize, Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive, Message,
)]
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
    let mc = tokio::spawn(async move {
        let mc: multi_channel::ProtobufChannel<Dummy> = multi_channel::channel_on(ADDR)
            .limit(LEN)
            .accept_full()
            .await?;

        let (rx, tx) = mc.split();
        let mut rx: mpsc::ProtobufReceiver<Dummy> = rx;
        let mut tx: broadcast::ProtobufSender<Dummy> = tx;

        // broadcast data concurrently
        let tx_handle = tokio::spawn(async move {
            let now = Instant::now();
            for i in 0..COUNT {
                let dummy = Dummy {
                    field1: "hello world".to_string(),
                    field2: 123213,
                    field3: i as u64,
                };

                tx.send(dummy).await?;
            }
            let duration = Instant::now() - now;
            info!("broadcasting {} msgs took {:?}", COUNT, duration);

            Ok::<_, Report>(())
        });

        // consume data concurrently
        let rx_handle = tokio::spawn(async move {
            let mut i = 0;
            let now = Instant::now();
            while let Some((item, _addr)) = rx.recv_with_addr().await {
                let _item = item?;

                i += 1;

                if i % (COUNT * LEN / 10) == 0 {
                    // info!("receiver: received {i} msgs from {addr}: {item:?}");
                    if i % (COUNT * LEN) == 0 {
                        break;
                    }
                }
            }
            let duration = Instant::now() - now;
            info!("rx: receiving {} msgs took {:?}", COUNT * LEN, duration);

            Ok::<_, Report>(())
        });

        tx_handle.await??;
        rx_handle.await??;

        Ok::<_, color_eyre::Report>(())
    });

    let handles = (0..LEN)
        .map(|n| {
            tokio::spawn(async move {
                let channel: channel::ProtobufChannel<Dummy> = channel::channel_to(ADDR)
                    .retry(Duration::from_millis(500), 100)
                    .await?;

                let (mut rx, mut tx) = channel.split();

                let mut i = 0;

                let rx_handle = tokio::spawn(async move {
                    let now = Instant::now();
                    while let Some(Ok(_)) = rx.recv().await {
                        i += 1;

                        if i % (COUNT / 10) == 0 {
                            info!("{n}: received {i} msgs");
                            if i % COUNT == 0 {
                                break;
                            }
                        }
                    }
                    let duration = Instant::now() - now;

                    info!("{n}: receiving {COUNT} msgs took {duration:?}");
                });

                let tx_handle = tokio::spawn(async move {
                    let now = Instant::now();
                    let mut i = 0;

                    while i < COUNT {
                        let dummy = Dummy {
                            field1: "hello world".to_string(),
                            field2: 123213,
                            field3: i as u64,
                        };

                        tx.send(dummy).await?;

                        i += 1;

                        if i % (COUNT / 10) == 0 {
                            info!("{n}: sent {i} msgs");
                            if i % COUNT == 0 {
                                break;
                            }
                        }
                    }
                    let duration = Instant::now() - now;
                    info!("{n}: sending {COUNT} msgs took {duration:?}");
                    Ok::<_, Report>(())
                });

                rx_handle.await?;
                tx_handle.await??;

                Ok::<_, color_eyre::Report>(())
            })
        })
        .collect::<Vec<_>>();

    mc.await??;
    try_join_all(handles)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    Ok(())
}
