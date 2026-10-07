//! The `lmx.v1.Owner` service.

use std::{sync::Arc, time::UNIX_EPOCH};

use lmx_ipc::proto::{
    self, ReserveRequest, ReserveResponse, StatusRequest, StatusResponse, reserve_response::Outcome,
};
use lmx_model::{ErrorBody, ErrorCode, Operation, Owner};
use serde_json::Map;
use solti::{
    api::ApiIdentity,
    core::SupervisorApi,
    model::{TaskQuery, TaskWorkload},
};
use tonic::{Request, Response, Status};

use crate::{
    auth,
    store::{self, Store},
    tasks::API_VERSION,
};

/// Handler of `lmx.v1.Owner`.
pub(crate) struct OwnerService {
    /// Store operations.
    pub(crate) store: Arc<Store>,
    /// Supervisor whose tasks are the operations.
    pub(crate) supervisor: Arc<SupervisorApi>,
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

#[tonic::async_trait]
impl proto::owner_server::Owner for OwnerService {
    async fn status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let owner = Owner {
            version: env!("CARGO_PKG_VERSION").into(),
            conditions: self.store.conditions(),
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
            Outcome::Failure(
                ErrorBody {
                    code: ErrorCode::PermissionDenied,
                    message: "Only root may reserve room in the store.".into(),
                    details: Map::new(),
                }
                .into(),
            )
        };
        Ok(Response::new(ReserveResponse {
            outcome: Some(outcome),
        }))
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
            (workload.api_version() == API_VERSION && store::runs(task.status())).then(|| {
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
