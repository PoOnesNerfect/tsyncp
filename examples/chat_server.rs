use color_eyre::{Report, Result};
use env_logger::Env;
use futures::future::try_join_all;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tsyncp::{broadcast, channel, mpsc, multi_channel};

const ADDR: &str = "localhost:8000";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chat {
    name: String,
    body: String,
}

#[derive(Debug, Clone)]
enum Message {
    Chat(Chat, SocketAddr),
    New(SocketAddr),
}

impl Message {
    pub fn chat(name: String, body: String, addr: SocketAddr) -> Self {
        Self::Chat(Chat { name, body }, addr)
    }

    pub fn new_joined(addr: SocketAddr) -> Self {
        Self::New(addr)
    }

    pub fn is_chat(&self) -> bool {
        matches!(self, Message::Chat(..))
    }

    pub fn is_new(&self) -> bool {
        !self.is_chat()
    }

    pub fn get_chat(self) -> Option<(Chat, SocketAddr)> {
        if let Self::Chat(chat, addr) = self {
            Some((chat, addr))
        } else {
            None
        }
    }

    pub fn get_new(self) -> Option<SocketAddr> {
        if let Self::New(addr) = self {
            Some(addr)
        } else {
            None
        }
    }
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
    let mc: multi_channel::JsonChannel<Chat> = multi_channel::channel_on(ADDR).accept(1).await?;

    let (receiver, sender) = mc.split();
    let mut receiver: mpsc::JsonReceiver<Chat> = receiver;
    let mut sender: broadcast::JsonSender<Chat> = sender;

    let (tx, mut rx) = tokio::sync::mpsc::channel(1024);

    // broadcast data concurrently
    let tx_handle = tokio::spawn(async move {
        loop {
            let (item, addrs) = receiver
                .accept()
                .until(
                    (),
                    |mut ch, _| async move { (ch.recv_with_addr().await, ch) },
                )
                .await;

            for addr in addrs {
                tx.send(Message::new_joined(addr)).await?;
            }

            if let Some(item) = item {
                let (chat, addr) = item?;

                println!("{}: {}\n", chat.name, chat.body);
                tx.send(Message::chat(chat.name, chat.body, addr)).await?;
            }
        }

        Ok::<_, Report>(())
    });

    // consume data concurrently
    let rx_handle = tokio::spawn(async move {
        loop {
            sender
                .accept()
                .until(&mut rx, |mut ch, rx| async move {
                    if let Some(msg) = rx.recv().await {
                        if msg.is_new() {
                            let addr = msg.get_new().unwrap();
                            println!("{addr} joined the chat!");
                            (Ok(()), ch)
                        } else {
                            let (chat, addr) = msg.get_chat().unwrap();
                            let res = ch.send_filtered(chat, |a| *a != addr).await;
                            (res, ch)
                        }
                    } else {
                        (Ok(()), ch)
                    }
                })
                .await
                .0?;
        }

        Ok::<_, Report>(())
    });

    tx_handle.await??;
    rx_handle.await??;

    Ok(())
}
