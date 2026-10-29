//! Who may do what.
//!
//! The socket is open to every local user. Each request carries the peer credentials of its
//! connection, and [`PeerIdentity`] puts the caller's user into the request as a [`Peer`]. Reading
//! is open to everyone; changing the system is for privileged callers: root, and the user `lmxd`
//! runs as, who gains nothing by asking it.

use tonic::{Request, Status, service::Interceptor, transport::server::UdsConnectInfo};

/// The user of the process at the other end of a request's connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Peer {
    /// Its user ID.
    pub(crate) uid: u32,
}

/// Puts the caller's peer credentials into each request as its [`Peer`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerIdentity;

impl Interceptor for PeerIdentity {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let credentials = request
            .extensions()
            .get::<UdsConnectInfo>()
            .and_then(|info| info.peer_cred)
            .ok_or_else(|| Status::unauthenticated("peer credentials are unavailable"))?;
        request.extensions_mut().insert(Peer {
            uid: credentials.uid(),
        });
        Ok(request)
    }
}

/// Whether `peer` may change the system: root, or `own_uid`, the user `lmxd` runs as.
pub(crate) fn privileged(peer: Option<&Peer>, own_uid: u32) -> bool {
    peer.is_some_and(|peer| peer.uid == 0 || peer.uid == own_uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_the_daemons_own_user_are_privileged() {
        let peer = |uid: u32| Peer { uid };
        assert!(privileged(Some(&peer(0)), 1000));
        assert!(privileged(Some(&peer(1000)), 1000));
        assert!(!privileged(Some(&peer(501)), 1000));
        assert!(!privileged(None, 1000));
    }
}
