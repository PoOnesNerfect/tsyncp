use color_eyre::Result;
use env_logger::Env;
use futures::future::try_join_all;
use log::{error, info};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tsyncp::broadcast;

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
        let mut receiever: broadcast::ProtobufSender<Dummy> = broadcast::broadcast_on(ADDR)
            .limit(LEN)
            .accept_full()
            .await?;

        // let (mut receiever, _sender) = receiever.split()?;

        let now = Instant::now();
        for i in 0..COUNT {
            receiever
                .broadcast(Dummy {
                    field1: "hello world".to_string(),
                    field2: 123213,
                    field3: i as u64,
                })
                .await?;
        }
        let duration = Instant::now() - now;
        info!("sending {} msgs took {:?}", COUNT, duration);

        Ok::<_, color_eyre::Report>(())
    });

    let handles = (0..LEN)
        .map(|n| {
            tokio::spawn(async move {
                let mut sender: broadcast::ProtobufReceiver<Dummy> = broadcast::listen_to(ADDR)
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
