//! Who may do what.
//!
//! The socket is open to every local user. Each request carries the peer credentials of its
//! connection, and [`PeerIdentity`] turns them into the Solti identity `uid:<uid>`. Reading is open
//! to everyone; changing the system is for privileged callers: root, and the user `lmxd` runs as,
//! who gains nothing by asking it.

use solti::{
    api::{ApiAuthorizer, ApiError, ApiIdentity, AuthorizationRequest, TaskOperation},
    runner::async_trait,
};
use tonic::{Request, Status, service::Interceptor, transport::server::UdsConnectInfo};

/// Puts the caller's peer credentials into each request as its [`ApiIdentity`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerIdentity;

impl Interceptor for PeerIdentity {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let credentials = request
            .extensions()
            .get::<UdsConnectInfo>()
            .and_then(|info| info.peer_cred)
            .ok_or_else(|| Status::unauthenticated("peer credentials are unavailable"))?;
        let mut identity = ApiIdentity::for_subject(format!("uid:{}", credentials.uid()))
            .with_attribute("uid", credentials.uid().to_string())
            .with_attribute("gid", credentials.gid().to_string());
        if let Some(pid) = credentials.pid() {
            identity = identity.with_attribute("pid", pid.to_string());
        }
        request.extensions_mut().insert(identity);
        Ok(request)
    }
}

/// Whether `identity` may change the system: root, or `own_uid`, the user `lmxd` runs as.
pub(crate) fn privileged(identity: Option<&ApiIdentity>, own_uid: u32) -> bool {
    identity
        .and_then(ApiIdentity::subject)
        .and_then(|subject| subject.strip_prefix("uid:"))
        .and_then(|uid| uid.parse::<u32>().ok())
        .is_some_and(|uid| uid == 0 || uid == own_uid)
}

/// Access to the Solti Task API: everyone reads, privileged callers cancel and delete, and nobody
/// creates or applies, because `lmxd` alone creates its tasks.
#[derive(Debug)]
pub(crate) struct TaskAccess {
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

#[async_trait]
impl ApiAuthorizer for TaskAccess {
    async fn authorize(&self, request: AuthorizationRequest<'_>) -> Result<(), ApiError> {
        match request.operation() {
            TaskOperation::Get
            | TaskOperation::List
            | TaskOperation::Watch
            | TaskOperation::ListRuns
            | TaskOperation::StreamLogs => Ok(()),
            TaskOperation::Cancel | TaskOperation::Delete
                if privileged(request.identity(), self.own_uid) =>
            {
                Ok(())
            }
            TaskOperation::Cancel | TaskOperation::Delete => Err(ApiError::Forbidden(
                "only root may cancel or delete tasks of lmxd".into(),
            )),
            _ => Err(ApiError::Forbidden(
                "tasks of lmxd are created by lmxd only".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use solti::{api::TaskTarget, model::TaskId};

    use super::*;

    #[test]
    fn root_and_the_daemons_own_user_are_privileged() {
        let identity = |uid: u32| ApiIdentity::for_subject(format!("uid:{uid}"));
        assert!(privileged(Some(&identity(0)), 1000));
        assert!(privileged(Some(&identity(1000)), 1000));
        assert!(!privileged(Some(&identity(501)), 1000));
        assert!(!privileged(
            Some(&ApiIdentity::for_subject("token:ci")),
            1000
        ));
        assert!(!privileged(None, 1000));
    }

    #[tokio::test]
    async fn everyone_reads_root_cancels_and_nobody_creates_tasks() {
        let access = TaskAccess { own_uid: 1000 };
        let root = ApiIdentity::for_subject("uid:0");
        let user = ApiIdentity::for_subject("uid:501");
        let name = TaskId::new("store-collect-1").expect("valid name");
        let allowed = async |identity: &ApiIdentity, operation| {
            let request =
                AuthorizationRequest::new(Some(identity), operation, TaskTarget::Task(&name));
            access.authorize(request).await.is_ok()
        };

        for operation in [
            TaskOperation::Get,
            TaskOperation::List,
            TaskOperation::Watch,
            TaskOperation::ListRuns,
            TaskOperation::StreamLogs,
        ] {
            assert!(allowed(&user, operation).await, "{operation:?}");
        }
        for operation in [TaskOperation::Cancel, TaskOperation::Delete] {
            assert!(allowed(&root, operation).await, "{operation:?}");
            assert!(!allowed(&user, operation).await, "{operation:?}");
        }
        for operation in [TaskOperation::Create, TaskOperation::Apply] {
            assert!(!allowed(&root, operation).await, "{operation:?}");
        }
    }
}
