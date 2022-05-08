use color_eyre::{Report, Result};
use env_logger::Env;
use log::error;
use serde::{Deserialize, Serialize};
use std::io;
use std::time::Duration;
use tsyncp::channel;

const ADDR: &str = "localhost:8000";

#[derive(Debug, Serialize, Deserialize)]
struct Chat {
    name: String,
    body: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let name = get_name()?;

    println!("\nEntering chatroom...");

    let ch: channel::JsonChannel<Chat> = channel::channel_to(ADDR)
        .retry(Duration::from_millis(500), 100)
        .await?;

    let (mut rx, mut tx) = ch.split();

    println!("Say something!");

    // read chats from server!
    tokio::spawn(async move {
        while let Some(chat) = rx.recv().await {
            let chat = chat?;
            println!("{}: {}", chat.name, chat.body);
        }

        Ok::<_, Report>(())
    });

    let mut buffer = String::new();

    while let Ok(_) = io::stdin().read_line(&mut buffer) {
        let chat = Chat {
            name: name.clone(),
            body: buffer.trim_end().to_string(),
        };

        tx.send(chat).await?;
        buffer.clear();
    }

    error!("Exiting chatroom due to stdio error");

    Ok(())
}

fn get_name() -> Result<String> {
    let mut buffer = String::new();

    println!("Enter your name:");
    io::stdin().read_line(&mut buffer)?;

    Ok(buffer.trim_end().to_string())
}
