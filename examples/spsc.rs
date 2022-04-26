use color_eyre::Result;
use env_logger::Env;
use log::{error, info};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tsyncp::spsc;

const ADDR1: &str = "localhost:8000";
const ADDR2: &str = "localhost:8001";

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
    }
}

async fn try_main() -> Result<()> {
    let sender_on_handle = tokio::spawn(async move {
        let mut sender: spsc::JsonSender<Dummy> = spsc::send_on(ADDR1).await?;
        let port = sender.local_addr().port();

        let dummy = Dummy {
            field1: String::from("hello world"),
            field2: 123123,
            field3: 123123123,
        };

        sender.send(dummy).await?;

        info!("{port} sent data");

        Ok::<_, color_eyre::Report>(())
    });

    let receiver_to_handle = tokio::spawn(async move {
        let mut receiver: spsc::JsonReceiver<Dummy> = spsc::recv_to(ADDR1)
            .retry(Duration::from_millis(500), 100)
            .await?;
        let port = receiver.local_addr().port();

        if let Some(item) = receiver.recv().await {
            let item = item?;
            info!("{port} received {item:?}");
        }

        Ok::<_, color_eyre::Report>(())
    });

    let sender_to_handle = tokio::spawn(async move {
        let mut sender: spsc::JsonSender<Dummy> = spsc::send_to(ADDR2)
            .retry(Duration::from_millis(500), 100)
            .await?;
        let port = sender.local_addr().port();

        let dummy = Dummy {
            field1: String::from("hello world"),
            field2: 123123,
            field3: 123123123,
        };

        sender.send(dummy).await?;

        info!("{port} sent data");

        Ok::<_, color_eyre::Report>(())
    });

    let receiver_on_handle = tokio::spawn(async move {
        let mut receiver: spsc::JsonReceiver<Dummy> = spsc::recv_on(ADDR2).await?;
        let port = receiver.local_addr().port();

        if let Some(item) = receiver.recv().await {
            let item = item?;
            info!("{port} received {item:?}");
        }

        Ok::<_, color_eyre::Report>(())
    });

    sender_on_handle.await??;
    receiver_to_handle.await??;
    sender_to_handle.await??;
    receiver_on_handle.await??;

    Ok(())
}
