use color_eyre::Result;
use env_logger::Env;
use futures::future::try_join_all;
use log::info;
use std::time::Duration;
use tsyncp::barrier;

const ADDR: &str = "localhost:8000";
const LEN: usize = 10;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let sender_handles = (0..LEN)
        .map(|_| {
            tokio::spawn(async move {
                let mut waiter: barrier::Waiter = barrier::waiter_to(ADDR)
                    .retry(Duration::from_millis(500), 100)
                    .await?;

                waiter.wait().await;

                let port = waiter.local_addr().port();
                info!("{port}: released");

                Ok::<_, color_eyre::Report>(())
            })
        })
        .collect::<Vec<_>>();

    let receiver_handle = tokio::spawn(async move {
        let mut barrier: barrier::Barrier = barrier::barrier_on(ADDR)
            .limit(LEN)
            .accept()
            .to_limit()
            .await?;

        for i in 0..5 {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            info!("waited {} seconds", i + 1);
        }

        barrier.release().await?;
        info!("barrier released");

        Ok::<_, color_eyre::Report>(())
    });

    try_join_all(sender_handles)
        .await?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    receiver_handle.await??;

    Ok(())
}
