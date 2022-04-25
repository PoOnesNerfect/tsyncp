use color_eyre::Result;
use env_logger::Env;
use log::{error, info};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tsyncp::channel;

const COUNT: u64 = 100_000;
const ADDR: &str = "localhost:8001";

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
        tokio::time::sleep(Duration::from_millis(5000)).await;

        let mut receiever: channel::ProtobufChannel<Dummy> = channel::channel_on(ADDR)
            .filter(|a| {
                info!("connected to {a}");
                true
            })
            .await?;
        // info!("Receiver: {:#?}", receiever);

        let now = Instant::now();
        for i in 0..COUNT {
            receiever
                .send(Dummy {
                    field1: "hello world".to_string(),
                    field2: 123213,
                    field3: i,
                })
                .await?;
        }
        let duration = Instant::now() - now;
        info!("sending {} msgs took {:?}", COUNT, duration);

        Ok::<_, color_eyre::Report>(())
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let sender_handle = tokio::spawn(async move {
        let mut sender: channel::ProtobufChannel<Dummy> = channel::channel_to(ADDR)
            .retry(Duration::from_millis(500), 100)
            .await?;

        let mut i = 0;

        let now = Instant::now();
        while let Some(Ok(_)) = sender.recv().await {
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

        Ok::<_, color_eyre::Report>(())
    });

    receiver_handle.await??;
    sender_handle.await??;

    Ok(())
}
