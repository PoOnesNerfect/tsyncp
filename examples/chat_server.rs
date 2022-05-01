use color_eyre::Result;
use env_logger::Env;
use serde::{Deserialize, Serialize};
use snafu::Error;
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

    let channel: multi_channel::JsonChannel<Chat> =
        multi_channel::channel_on(ADDR).accept(1).await?;

    let (mut rx, mut tx) = channel.split();

    // broadcast data concurrently
    while let (Some(res), addrs) = rx.recv().with_addr().accepting().await {
        let addrs = addrs?;

        let (chat, addr) = match res {
            Ok((chat, addr)) => (chat, addr),
            Err(e) => {
                if e.is_connection_reset()
                    || e.is_not_connected()
                    || e.is_connection_refused()
                    || e.is_connection_aborted()
                {
                    println!("{} disconnected!", e.addr());
                    continue;
                } else {
                    Err(e)?
                }
            }
        };

        for addr in addrs {
            println!("\n{addr} joined the chatroom!\n");
        }

        println!("{}: {}", chat.name, chat.body);
        let res = tx.send(chat).filtered(|a| a != addr).accepting().await.0;
        if let Err(e) = res {
            if let Some(errors) = e.errors() {
                for e in errors {
                    log::error!("e: {}", e.description());
                }
            }
        }
    }

    Ok(())
}
