//! Calls to the guest owner daemon `lmxd`.
//!
//! Owner operations run only in `lmxd`; `lmx` asks for them over the daemon's socket and never runs
//! them itself. A daemon that cannot be reached is reported, never replaced.

use std::{fmt, future::Future, path::Path, time::Duration};

use lmx_ipc::{
    proto::{ReserveRequest, StatusRequest, owner_client::OwnerClient},
    tonic::{Code, Status, transport::Channel},
};
use lmx_model::{ErrorBody, Owner, Reserve};

/// How long `lmx status` waits for `lmxd` to connect and answer.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long `lmx store reserve` waits for `lmxd` to answer at all before it asks for the reserve.
///
/// systemd accepts connections on the socket before the daemon runs, so a daemon that never
/// starts would otherwise keep the host waiting; this is long enough for a socket-activated start.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a call to `lmxd` gave no answer of the host contract.
#[derive(Debug)]
pub(crate) enum CallError {
    /// `lmxd` could not be reached, did not answer in time, or reported itself unavailable.
    Unavailable(String),
    /// `lmxd` answered outside its protocol, or the call could not be made.
    Failed(String),
}

impl CallError {
    /// Error of a call that ended with gRPC `status`.
    fn from_status(status: &Status) -> Self {
        let message = format!("lmxd did not answer: {}", status.message());
        if status.code() == Code::Unavailable {
            Self::Unavailable(message)
        } else {
            Self::Failed(message)
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Asks `lmxd` on `socket` for its state, waiting at most [`STATUS_TIMEOUT`].
pub(crate) fn status(socket: &Path) -> Result<Owner, CallError> {
    block_on(async {
        let call = async {
            let mut client = connect(socket).await?;
            let response = client
                .status(StatusRequest {})
                .await
                .map_err(|status| CallError::from_status(&status))?;
            Ok(Owner::from(response.into_inner()))
        };
        tokio::time::timeout(STATUS_TIMEOUT, call)
            .await
            .unwrap_or_else(|_| Err(silent(STATUS_TIMEOUT)))
    })
}

/// Asks `lmxd` on `socket` to make room in the store, waiting as long as the collection takes.
///
/// `lmxd` must first answer a status call within [`PROBE_TIMEOUT`], so a daemon that never starts is
/// reported as unavailable instead of waited for.
pub(crate) fn reserve(socket: &Path) -> Result<Result<Reserve, ErrorBody>, CallError> {
    block_on(async {
        let mut client = connect(socket).await?;
        tokio::time::timeout(PROBE_TIMEOUT, client.status(StatusRequest {}))
            .await
            .map_err(|_| silent(PROBE_TIMEOUT))?
            .map_err(|status| CallError::from_status(&status))?;
        let response = client
            .reserve(ReserveRequest {})
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::reserve_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Error of a daemon that did not answer within `timeout`.
fn silent(timeout: Duration) -> CallError {
    CallError::Unavailable(format!(
        "lmxd did not answer within {} seconds",
        timeout.as_secs()
    ))
}

/// Connects to `lmxd` on `socket`.
async fn connect(socket: &Path) -> Result<OwnerClient<Channel>, CallError> {
    lmx_ipc::connect(socket).await.map_err(|error| {
        CallError::Unavailable(format!(
            "lmxd is not reachable at {}: {error}",
            socket.display()
        ))
    })
}

/// Runs `call` on a runtime of its own; `lmx` makes one call per command.
fn block_on<T>(call: impl Future<Output = Result<T, CallError>>) -> Result<T, CallError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CallError::Failed(format!("cannot start the async runtime: {error}")))?
        .block_on(call)
}
