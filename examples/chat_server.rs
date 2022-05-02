use color_eyre::Result;
use env_logger::Env;
use log::error;
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

    let channel: multi_channel::JsonChannel<Chat> =
        multi_channel::channel_on(ADDR).accept(1).await?;

    let (mut rx, mut tx) = channel.split();

    // broadcast data concurrently
    while let (Some(res), accepted_addrs) = rx.recv().with_addr().accepting().await {
        let (chat, addr) = match res {
            Ok((chat, addr)) => (chat, addr),
            Err(e) => {
                if e.is_connection_error() {
                    println!("{} disconnected!", e.peer_addr().unwrap());
                    continue;
                } else {
                    Err(e)?
                }
            }
        };

        for a in accepted_addrs? {
            println!("\n{a} joined the chatroom!\n");
        }

        println!("{}: {}", chat.name, chat.body);

        let (res, accepted_addrs) = tx.send(chat).filtered(|a| a != addr).accepting().await;

        for a in accepted_addrs? {
            println!("\n{a} joined the chatroom!\n");
        }

        if let Err(error) = res {
            // if the error is from connection dropping, then just report disconnected,
            // if not, report it.
            for e in error.as_sink_errors() {
                if e.is_connection_error() {
                    println!("{} disconnected!", e.peer_addr());
                } else {
                    report(e, false);
                }
            }

            // if error is from encoding, then return it.
            if error.is_encode_error() {
                Err(error)?;
            }
        }
    }

    Ok(())
}

fn report<E: 'static>(err: &E, backtrace: bool)
where
    E: std::error::Error,
    E: snafu::ErrorCompat,
    E: Send + Sync,
{
    let mut error_str = format!("[ERROR] {}\n\n", err);

    if let Some(source) = err.source() {
        error_str.push_str("Caused by:\n");
        for (i, e) in std::iter::successors(Some(source), |e| e.source()).enumerate() {
            error_str.push_str(&format!("   {}: {}\n", i, e));
        }
    }

    if backtrace {
        if let Some(backtrace) = snafu::ErrorCompat::backtrace(err) {
            error_str.push_str("\nBacktrace:\n");
            error_str.push_str(&format!("{:?}", backtrace));
        }
    }

    error!("{}", error_str);
}
