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
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let ch: channel::JsonChannel<Chat> = channel::channel_to(ADDR)
        .retry(Duration::from_millis(500), 100)
        .await?;

    let (receiver, sender) = ch.split();
    let mut receiver: broadcast::JsonReceiver<Chat> = receiver;
    let mut sender: mpsc::JsonSender<Chat> = sender;

    let tx_handle = tokio::spawn(async move {
        let mut name = String::new();

        println!("Enter your name:");
        io::stdin().read_line(&mut name)?;
        name = name.trim_end().to_string();

        println!("\nEntering chatroom...");
        println!("Say something!");

        let mut buffer = String::new();
        while let Ok(_) = io::stdin().read_line(&mut buffer) {
            let chat = Chat {
                name: name.clone(),
                body: buffer.trim_end().to_string(),
            };

            sender.send(chat).await?;
            buffer.clear();
        }

        error!("Exiting chatroom due to stdio error");

        Ok::<_, Report>(())
    });

    let rx_handle = tokio::spawn(async move {
        while let Some(chat) = receiver.recv().await {
            let chat = chat?;
            println!("{}: {}", chat.name, chat.body);
        }

        Ok::<_, Report>(())
    });

    tx_handle.await??;
    rx_handle.await??;

    Ok(())
}
