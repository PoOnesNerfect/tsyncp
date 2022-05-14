use color_eyre::Result;
use env_logger::Env;
use log::info;
use prost::Message;
use std::time::{Duration, Instant};
use tsyncp::channel;

const COUNT: u64 = 100_000;
const ADDR: &str = "localhost:8001";

#[derive(Message)]
struct Dummy {
    #[prost(string, tag = "1")]
    field1: String,
    #[prost(uint64, tag = "2")]
    field2: u64,
    #[prost(uint64, tag = "3")]
    field3: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let handle1 = tokio::spawn(async move {
        let mut ch: channel::ProstChannel<Dummy> = channel::channel_on(ADDR)
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

        Ok::<_, color_eyre::Report>(())
    });

    let handle2 = tokio::spawn(async move {
        let mut ch: channel::ProstChannel<Dummy> = channel::channel_to(ADDR)
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

        Ok::<_, color_eyre::Report>(())
    });

    handle1.await??;
    handle2.await??;

    Ok(())
}
