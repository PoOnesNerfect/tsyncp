use color_eyre::Result;
use env_logger::Env;
use log::error;
use serde::{Deserialize, Serialize};
use tsyncp::multi_channel;

const ADDR: &str = "localhost:8000";

#[derive(Debug, Serialize, Deserialize)]
struct Chat {
    name: String,
    body: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let mut channel: multi_channel::JsonChannel<Chat> = multi_channel::channel_on(ADDR)
        .accept()
        .handle(|a| println!("\n{a} joined the chatroom!\n"))
        .await?;

    // listen to incoming chat messages and accept connections
    while let (Some(res), _) = channel
        .recv()
        .with_addr()
        .accepting()
        .handle(|a| println!("\n{a} joined the chatroom!\n"))
        .await
    {
        let (chat, addr) = match res {
            Ok((chat, addr)) => (chat, addr),
            Err(e) => {
                if e.is_connection_error() {
                    println!("{} disconnected!", e.peer_addr().unwrap());
                } else {
                    report(&e, true);
                }

                continue;
            }
        };

        // display chat message.
        println!("{}: {}", chat.name, chat.body);

        // broadcast chat message to all clients except where it came from.
        let (res, _) = channel
            .send(chat)
            .filter(|a| a != addr)
            .accepting()
            .handle(|a| println!("\n{a} joined the chatroom!\n"))
            .await;

        // handle error cases.
        if let Err(error) = res {
            // if error is from encoding, then report it.
            if error.is_encode_error() {
                report(&error, true);
                continue;
            }

            // if the error is from connection dropping, then display disconnected,
            // if not, report it.
            for e in error.as_sink_errors() {
                if e.is_connection_error() {
                    println!("{} disconnected!", e.peer_addr());
                } else {
                    report(e, false);
                }
            }
        }
    }

    Ok(())
}

fn report<E>(err: &E, backtrace: bool)
where
    E: 'static + Send + Sync + snafu::Error + snafu::ErrorCompat,
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
