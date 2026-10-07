//! The `lmx.v1.Owner` service.

use std::{sync::Arc, time::UNIX_EPOCH};

use lmx_ipc::{
    outcome_event,
    proto::{
        self, ApplyRequest, CancelApplyRequest, CancelApplyResponse, ReserveRequest,
        ReserveResponse, StatusRequest, StatusResponse, cancel_apply_response,
        reserve_response::Outcome,
    },
};
use lmx_model::{Apply, ApplyEvent, ApplyState, Condition, ErrorBody, ErrorCode, Operation, Owner};
use serde_json::Map;
use solti::{
    api::ApiIdentity,
    core::SupervisorApi,
    model::{TaskQuery, TaskWorkload},
};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    apply::{Applier, Joined, Message},
    auth, launch,
    observer::{self, Health, Observer},
    paths::Paths,
    store::Store,
    tasks::API_VERSION,
};

/// Events buffered for a follower whose connection is slow.
const FOLLOW_BUFFER: usize = 64;

/// Handler of `lmx.v1.Owner`.
pub(crate) struct OwnerService {
    /// Store operations.
    pub(crate) store: Arc<Store>,
    /// The apply operation.
    pub(crate) applier: Arc<Applier>,
    /// The generation observer; `None` in a transient daemon.
    pub(crate) observer: Option<Arc<Observer>>,
    /// Guest locations.
    pub(crate) paths: Paths,
    /// Supervisor whose tasks are the operations.
    pub(crate) supervisor: Arc<SupervisorApi>,
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

impl OwnerService {
    /// Conditions of the generations and the store disk now.
    fn conditions(&self) -> Vec<Condition> {
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        let kept = lmx_facts::generations::system_generations(&self.paths.profiles()).ok();
        let (health, finalizing) = self
            .observer
            .as_ref()
            .map_or((Health::Unknown, false), |observer| {
                (observer.health(), observer.finalizing())
            });
        let mut conditions = observer::conditions(&generations, kept, &health, finalizing);
        conditions.extend(self.store.conditions());
        conditions
    }
}

#[tonic::async_trait]
impl proto::owner_server::Owner for OwnerService {
    type ApplyStream = ReceiverStream<Result<proto::ApplyEvent, Status>>;

    async fn status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let owner = Owner {
            version: env!("CARGO_PKG_VERSION").into(),
            conditions: self.conditions(),
            operations: operations(&self.supervisor),
        };
        Ok(Response::new(owner.into()))
    }

    async fn reserve(
        &self,
        request: Request<ReserveRequest>,
    ) -> Result<Response<ReserveResponse>, Status> {
        let caller = request.extensions().get::<ApiIdentity>();
        let outcome = if auth::privileged(caller, self.own_uid) {
            match self.store.reserve().await {
                Ok(reserve) => Outcome::Result(reserve.into()),
                Err(error) => Outcome::Failure(error.into()),
            }
        } else {
            Outcome::Failure(denied("Only root may reserve room in the store.").into())
        };
        Ok(Response::new(ReserveResponse {
            outcome: Some(outcome),
        }))
    }

    async fn apply(
        &self,
        request: Request<ApplyRequest>,
    ) -> Result<Response<Self::ApplyStream>, Status> {
        let privileged = auth::privileged(request.extensions().get(), self.own_uid);
        let ApplyRequest { generation, follow } = request.into_inner();
        let (events, stream) = mpsc::channel(FOLLOW_BUFFER);
        let outcome = if privileged {
            match self.applier.apply(&generation).await {
                Ok(Joined::Following(messages)) if follow => {
                    tokio::spawn(forward(messages, events));
                    return Ok(Response::new(ReceiverStream::new(stream)));
                }
                Ok(Joined::Following(_)) => Ok(Apply {
                    generation,
                    state: ApplyState::Running,
                }),
                Ok(Joined::Done(outcome)) => outcome,
                Err(error) => Err(error),
            }
        } else {
            Err(denied("Only root may apply a generation."))
        };
        // The stream is new and its buffer empty, so the one event fits.
        let _ = events.try_send(Ok(outcome_event(outcome)));
        Ok(Response::new(ReceiverStream::new(stream)))
    }

    async fn cancel_apply(
        &self,
        request: Request<CancelApplyRequest>,
    ) -> Result<Response<CancelApplyResponse>, Status> {
        let privileged = auth::privileged(request.extensions().get(), self.own_uid);
        let outcome = if privileged {
            let cancelled = self.applier.cancel(&request.into_inner().generation).await;
            cancel_apply_response::Outcome::Cancelled(cancelled.cancelled)
        } else {
            cancel_apply_response::Outcome::Failure(denied("Only root may cancel an apply.").into())
        };
        Ok(Response::new(CancelApplyResponse {
            outcome: Some(outcome),
        }))
    }
}

/// Passes the messages of an apply to a follower until the outcome, or until it disconnects.
async fn forward(
    mut messages: broadcast::Receiver<Message>,
    events: mpsc::Sender<Result<proto::ApplyEvent, Status>>,
) {
    loop {
        let event = match messages.recv().await {
            Ok(Message::Event(event)) => event.into(),
            Ok(Message::Outcome(outcome)) => {
                let _ = events.send(Ok(outcome_event(outcome))).await;
                return;
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                ApplyEvent::Lagged { skipped }.into()
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if events.send(Ok(event)).await.is_err() {
            return;
        }
    }
}

/// The failure of a caller who may not change the system.
fn denied(message: &str) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::PermissionDenied,
        message: message.into(),
        details: Map::new(),
    }
}

/// Pending and running tasks of `lmxd`, without those their runner could not build.
fn operations(supervisor: &SupervisorApi) -> Vec<Operation> {
    let Ok(page) = supervisor.query_tasks(&TaskQuery::new().with_active()) else {
        return Vec::new();
    };
    page.items
        .iter()
        .filter_map(|task| {
            let TaskWorkload::Extension(workload) = task.spec().workload() else {
                return None;
            };
            (workload.api_version() == API_VERSION && launch::runs(task.status())).then(|| {
                Operation {
                    task: task.name().to_string(),
                    kind: workload.kind().to_owned(),
                    phase: task.status().phase().to_string(),
                    created_at: task
                        .metadata()
                        .creation_timestamp()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |since| {
                            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                        }),
                }
            })
        })
        .collect()
}
