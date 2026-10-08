//! Connection to `lmxd` over its Unix socket.

use std::{io, path::Path};

use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::proto::owner_client::OwnerClient;

/// Connects to `lmxd` on the socket at `path`.
///
/// The socket is connected before the client is built, and a missing socket or a refused connection
/// is reported with the operating system's reason. Callers bound their first call: a socket that
/// systemd holds for a daemon that does not run yet accepts the connection. The connection is used
/// once: a client that loses it fails its next call instead of reconnecting.
pub async fn connect(path: &Path) -> io::Result<OwnerClient<Channel>> {
    let mut stream = Some(UnixStream::connect(path).await?);
    // The URI only names the peer in HTTP/2 requests; the connector ignores it.
    let channel = Endpoint::from_static("http://lmxd")
        .connect_with_connector(service_fn(move |_: Uri| {
            let stream = stream.take();
            async move {
                stream.map(TokioIo::new).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "the lmxd connection was lost")
                })
            }
        }))
        .await
        .map_err(io::Error::other)?;
    Ok(OwnerClient::new(channel))
}
