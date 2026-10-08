//! Calls to the guest owner daemon `lmxd`.
//!
//! Owner operations run only in `lmxd`; `lmx` asks for them over the daemon's socket and never runs
//! them itself. A daemon that cannot be reached is reported, never replaced.

use std::{fmt, future::Future, io, path::Path, process::ExitCode, time::Duration};

use lmx_ipc::{
    ApplyMessage,
    proto::{
        ApplyRequest, CancelApplyRequest, ReserveRequest, StatusRequest, owner_client::OwnerClient,
    },
    tonic::{Code, Status, transport::Channel},
};
use lmx_model::{Apply, ApplyEvent, CancelApply, ErrorBody, ErrorCode, Owner, Reserve};
use serde_json::Map;

use crate::output;

/// How long `lmx status` waits for `lmxd` to connect and answer.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long an owner operation waits for `lmxd` to answer at all before it asks for the operation.
///
/// systemd accepts connections on the socket before the daemon runs, and without this limit a
/// daemon that never starts would keep the host waiting; this is long enough for a
/// socket-activated start.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a call to `lmxd` gave no answer of the host contract.
#[derive(Debug)]
pub(crate) enum CallError {
    /// `lmxd` could not be reached, did not answer in time, or reported itself unavailable.
    Unavailable(String),
    /// `lmxd` answered outside its protocol, or the call could not be made.
    Failed(String),
    /// An answer could not be passed on, such as an event to a closed standard output.
    Output(io::Error),
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
            Self::Output(error) => write!(formatter, "{error}"),
        }
    }
}

/// Asks `lmxd` on `socket` for its state, waiting at most [`STATUS_TIMEOUT`].
pub(crate) fn status(socket: &Path) -> Result<Owner, CallError> {
    status_within(socket, STATUS_TIMEOUT)
}

/// Asks `lmxd` on `socket` for its state, waiting at most `timeout` to connect and get the answer.
pub(crate) fn status_within(socket: &Path, timeout: Duration) -> Result<Owner, CallError> {
    block_on(async {
        let call = async {
            let mut client = connect(socket).await?;
            let response = client
                .status(StatusRequest {})
                .await
                .map_err(|status| CallError::from_status(&status))?;
            Ok(Owner::from(response.into_inner()))
        };
        tokio::time::timeout(timeout, call)
            .await
            .unwrap_or_else(|_| Err(silent(timeout)))
    })
}

/// Asks `lmxd` on `socket` to make room in the store, waiting as long as the collection takes.
pub(crate) fn reserve(socket: &Path) -> Result<Result<Reserve, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let response = client
            .reserve(ReserveRequest {})
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::reserve_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Asks `lmxd` on `socket` to apply `generation`, passing each event to `event` until the outcome.
///
/// Without `follow`, `lmxd` answers at once that the apply runs or that the generation is built.
/// The apply belongs to `lmxd`: a caller that stops following leaves it running.
pub(crate) fn apply(
    socket: &Path,
    generation: &str,
    follow: bool,
    mut event: impl FnMut(ApplyEvent) -> io::Result<()>,
) -> Result<Result<Apply, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let request = ApplyRequest {
            generation: generation.to_owned(),
            follow,
        };
        let mut events = client
            .apply(request)
            .await
            .map_err(|status| CallError::from_status(&status))?
            .into_inner();

        loop {
            let message = events
                .message()
                .await
                .map_err(|status| {
                    CallError::Unavailable(format!(
                        "lmxd stopped answering during the apply: {}",
                        status.message()
                    ))
                })?
                .ok_or_else(|| {
                    CallError::Unavailable("lmxd ended the apply without an outcome".into())
                })?;
            match lmx_ipc::apply_message(message)
                .map_err(|error| CallError::Failed(error.to_string()))?
            {
                ApplyMessage::Event(update) => event(update).map_err(CallError::Output)?,
                ApplyMessage::Outcome(outcome) => return Ok(outcome),
            }
        }
    })
}

/// Asks `lmxd` on `socket` to cancel the apply of `generation`; it answers once the apply stopped.
pub(crate) fn cancel_apply(
    socket: &Path,
    generation: &str,
) -> Result<Result<CancelApply, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let response = client
            .cancel_apply(CancelApplyRequest {
                generation: generation.to_owned(),
            })
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::cancel_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Reports a call that gave no answer of the host contract.
///
/// An unreachable daemon is the contract's `owner.unavailable`, with exit status 3. A broken call
/// has no contract code and is reported on standard error only.
pub(crate) fn report(error: CallError, json: bool) -> io::Result<ExitCode> {
    match error {
        CallError::Unavailable(message) => output::failure(
            json,
            ErrorBody {
                code: ErrorCode::OwnerUnavailable,
                message,
                details: Map::new(),
            },
            output::UNAVAILABLE,
        ),
        CallError::Failed(message) => {
            eprintln!("lmx: {message}");
            Ok(ExitCode::from(output::FAILURE))
        }
        CallError::Output(error) => Err(error),
    }
}

/// Connects to `lmxd` on `socket` and checks that it answers within [`PROBE_TIMEOUT`]: a daemon
/// that never starts is reported as unavailable instead of waited for.
async fn ready(socket: &Path) -> Result<OwnerClient<Channel>, CallError> {
    let mut client = connect(socket).await?;
    tokio::time::timeout(PROBE_TIMEOUT, client.status(StatusRequest {}))
        .await
        .map_err(|_| silent(PROBE_TIMEOUT))?
        .map_err(|status| CallError::from_status(&status))?;
    Ok(client)
}

/// Error of a daemon that did not answer within `timeout`.
fn silent(timeout: Duration) -> CallError {
    let span = if timeout.subsec_millis() == 0 {
        format!("{} seconds", timeout.as_secs())
    } else {
        format!("{} ms", timeout.as_millis())
    };
    CallError::Unavailable(format!("lmxd did not answer within {span}"))
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
