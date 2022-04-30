use super::TcpStreamSettings;
use futures::ready;
use std::fmt;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use tokio::net::TcpListener;
use tokio::net::TcpStream;

pub trait Accept {
    type Output: fmt::Debug;
    type Config: ConfigTrait;
    type Error: 'static + snafu::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>>;

    fn handle_abrupt_drop(&self) {}
}

pub trait ConfigTrait: fmt::Debug + PartialEq + Clone {}
impl<T: fmt::Debug + PartialEq + Clone> ConfigTrait for T {}

impl Accept for TcpListener {
    type Output = TcpStream;
    type Config = TcpStreamSettings;
    type Error = std::io::Error;

    fn poll_accept(
        &self,
        config: &Self::Config,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(Self::Output, SocketAddr), Self::Error>> {
        let (o, i) = match ready!(self.poll_accept(cx)) {
            Ok((o, i)) => (o, i),
            Err(error) => return Poll::Ready(Err(error)),
        };

        if let Some(nodelay) = config.nodelay {
            o.set_nodelay(nodelay)?;
        }

        if let Some(ttl) = config.ttl {
            o.set_ttl(ttl)?;
        }

        Poll::Ready(Ok((o, i)))
    }
}
