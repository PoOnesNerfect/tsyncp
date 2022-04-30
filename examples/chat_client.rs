use color_eyre::{Report, Result};
use env_logger::Env;
use log::error;
use serde::{Deserialize, Serialize};
use std::io;
use std::time::Duration;
use tsyncp::{broadcast, channel, mpsc};

const ADDR: &str = "localhost:8000";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chat {
    name: String,
    body: String,
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
    let ch: channel::JsonChannel<Chat> = channel::channel_to(ADDR)
        .retry(Duration::from_millis(500), 100)
        .await?;

    let (receiver, sender) = ch.split();
    let mut receiver: broadcast::JsonReceiver<Chat> = receiver;
    let mut sender: mpsc::JsonSender<Chat> = sender;

    // broadcast data concurrently
    let tx_handle = tokio::spawn(async move {
        println!("Your name:");
        let mut name = String::new();
        io::stdin().read_line(&mut name)?;
        name = name.trim_end().to_string();
        println!("\nEntering chat");

        let mut buffer = String::new();
        loop {
            io::stdin().read_line(&mut buffer)?;
            println!("\n");

            let chat = Chat {
                name: name.clone(),
                body: buffer.trim_end().to_string(),
            };

            sender.send(chat).await?;
        }

        Ok::<_, Report>(())
    });

    // consume data concurrently
    let rx_handle = tokio::spawn(async move {
        while let Some(chat) = receiver.recv().await {
            let chat = chat?;
            println!("{}: {}\n", chat.name, chat.body);
        }

        Ok::<_, Report>(())
    });

    tx_handle.await??;
    rx_handle.await??;

    Ok(())
}
