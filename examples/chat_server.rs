use color_eyre::Result;
use env_logger::Env;
use serde::{Deserialize, Serialize};
use tsyncp::multi_channel;

const ADDR: &str = "localhost:8000";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chat {
    name: String,
    body: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let mut channel: multi_channel::JsonChannel<Chat> =
        multi_channel::channel_on(ADDR).accept(1).await?;

    // broadcast data concurrently
    while let (Some(res), addrs) = channel.recv().with_addr().accepting().await {
        let addrs = addrs?;
        let (chat, addr) = res?;

        for addr in addrs {
            println!("\n{addr} joined the chatroom!\n");
        }

        println!("{}: {}", chat.name, chat.body);
        channel.send_filtered(chat, |a| a != &addr).await?;
    }

    Ok(())
}
