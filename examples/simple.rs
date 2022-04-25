use color_eyre::Result;
use env_logger::Env;
use futures::future::try_join_all;
use log::{error, info};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tsyncp::{channel, multi_channel};

const COUNT: usize = 100_000;
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
    let receiver_handle = tokio::spawn(async move {
        let mut receiever: multi_channel::ProtobufChannel<Dummy> = multi_channel::channel_on(ADDR)
            .limit(LEN)
            .accept_full()
            .await?;
        // info!("Receiver: {:#?}", receiever);

        let addrs = receiever.peer_addrs();

        let now = Instant::now();
        for i in 0..COUNT {
            receiever
                .send_to(
                    Dummy {
                        field1: "hello world".to_string(),
                        field2: 123213,
                        field3: i as u64,
                    },
                    &addrs,
                )
                .await?;
        }
        let duration = Instant::now() - now;
        info!("sending {} msgs took {:?}", COUNT, duration);

        let mut i = 0;
        let now = Instant::now();
        while let Some((item, addr)) = receiever.recv_with_addr().await {
            let item = item?;

            i += 1;

            if i % (COUNT * LEN / 10) == 0 {
                // info!("receiver: received {i} msgs from {addr}: {item:?}");
                if i % (COUNT * LEN) == 0 {
                    break;
                }
            }
        }
        let duration = Instant::now() - now;
        info!(
            "receiver: receiving {} msgs took {:?}",
            COUNT * LEN,
            duration
        );

        let addrs = receiever.peer_addrs();
        info!("addrs: {addrs:?}");

        Ok::<_, color_eyre::Report>(())
    });

    let handles = (0..LEN)
        .map(|n| {
            tokio::spawn(async move {
                let mut sender: channel::ProtobufChannel<Dummy> = channel::channel_to(ADDR)
                    .retry(Duration::from_millis(500), 100)
                    .await?;
                info!("{n}: addr: {}", sender.local_addr());

                let mut i = 0;

                let now = Instant::now();
                while let Some(Ok(_)) = sender.recv().await {
                    i += 1;

                    if i % (COUNT / 10) == 0 {
                        // info!("{n}: received {i} msgs");
                        if i % COUNT == 0 {
                            break;
                        }
                    }
                }
                let duration = Instant::now() - now;

                info!("{n}: receiving {COUNT} msgs took {duration:?}");

                let now = Instant::now();
                let mut i = 0;

                while i < COUNT {
                    sender
                        .send(Dummy {
                            field1: "hello world".to_string(),
                            field2: 123213,
                            field3: i as u64,
                        })
                        .await?;

                    i += 1;

                    if i % (COUNT / 10) == 0 {
                        // info!("{n}: sent {i} msgs");
                        if i % COUNT == 0 {
                            break;
                        }
                    }
                }
                let duration = Instant::now() - now;
                info!("{n}: sending {COUNT} msgs took {duration:?}");

                Ok::<_, color_eyre::Report>(())
            })
        })
        .collect::<Vec<_>>();

    receiver_handle.await??;
    try_join_all(handles)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    Ok(())
}
