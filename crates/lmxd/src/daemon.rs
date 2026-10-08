//! Starting `lmxd` and serving its socket.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use lmx_ipc::proto::owner_server::OwnerServer;
use lmx_model::Config;
use solti::{
    api::{GrpcApi, SupervisorApiAdapter},
    core::{CoreError, SupervisorApi},
    exec::subprocess::SubprocessRunner,
    runner::RunnerRouter,
};
use tokio::{net::UnixListener, sync::Notify, task::JoinHandle};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{service::interceptor::InterceptedService, transport::Server};

use crate::{
    apply::Applier,
    auth::{PeerIdentity, TaskAccess},
    capture::Capture,
    journal::Journal,
    observer::{self, Observer, ObserverSchedule},
    owner::OwnerService,
    paths::Paths,
    store::{self, GuardSchedule, Store, UsageSource},
    tasks::{self, RegisterError, Setup},
};

/// Longest wait for open connections, such as a client following a stream, when stopping.
const DRAIN: Duration = Duration::from_secs(5);

/// Longest wait for tasks to stop.
const STOP_TASKS: Duration = Duration::from_secs(10);

/// Longest wait for task processes to be cleaned up after the tasks stopped.
const STOP_PROCESSES: Duration = Duration::from_secs(5);

/// What `lmxd` needs besides its socket.
pub struct Options {
    /// Platform configuration: disk thresholds and tool paths.
    pub config: Config,
    /// Reads the usage of the store file system.
    pub usage: UsageSource,
    /// When the store guard checks the disk; `None` turns the guard off.
    pub guard: Option<GuardSchedule>,
    /// When the generation observer looks at the booted generation; `None` turns it off, as in the
    /// transient daemon of an update.
    pub observer: Option<ObserverSchedule>,
    /// System root of the guest paths besides the store and the socket: `/` in a guest, a prepared
    /// tree in tests.
    pub root: PathBuf,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Options")
            .field("config", &self.config)
            .field("guard", &self.guard)
            .field("observer", &self.observer)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Failure to start or to serve.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The configuration cannot work.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// The runners could not be registered.
    #[error(transparent)]
    Register(#[from] RegisterError),
    /// The supervisor could not start or stop.
    #[error("supervisor: {0}")]
    Supervisor(#[from] CoreError),
    /// The socket could not be served.
    #[error("serving the socket failed: {0}")]
    Serve(#[from] tonic::transport::Error),
    /// The server stopped by itself, such as after a panic.
    #[error("the server stopped unexpectedly: {0}")]
    Stopped(String),
}

/// A started daemon: tasks run, and the socket waits to be served.
pub struct Daemon {
    /// Supervisor of all tasks.
    supervisor: Arc<SupervisorApi>,
    /// Private runner of task processes; stopped after the supervisor.
    subprocess: Arc<SubprocessRunner>,
    /// Store operations.
    store: Arc<Store>,
    /// The apply operation.
    applier: Arc<Applier>,
    /// The generation observer, when it runs.
    observer: Option<Arc<Observer>>,
    /// Guest locations.
    paths: Paths,
    /// The store guard and the observer loop, while they run.
    background: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Daemon {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Daemon")
            .field("paths", &self.paths)
            .field("observer", &self.observer.is_some())
            .finish_non_exhaustive()
    }
}

impl Daemon {
    /// Starts the supervisor, the runners and, when scheduled, the store guard and the generation
    /// observer.
    pub async fn start(options: Options) -> Result<Self, Error> {
        let Options {
            config,
            usage,
            guard,
            observer: observing,
            root,
        } = options;
        check(&config)?;
        let paths = Paths::new(root);
        let capture = Arc::new(Capture::default());
        let setup = Setup {
            tools: config.tools.clone(),
            paths: paths.clone(),
            user: config.user.name.clone(),
            units: config.health.units.clone(),
        };
        let mut router = RunnerRouter::new();
        let subprocess = tasks::register(&mut router, setup, Arc::clone(&capture))?;
        let supervisor = Arc::new(
            SupervisorApi::builder(router)
                .with_output_sink(Arc::new(Journal))
                .start()
                .await?,
        );
        let store = Arc::new(Store::new(Arc::clone(&supervisor), config.disk, usage));
        let applier = Arc::new(Applier::new(
            Arc::clone(&supervisor),
            Arc::clone(&store),
            Arc::clone(&capture),
            paths.clone(),
            config.user.gid,
        ));

        let mut background = Vec::new();
        if let Some(schedule) = guard {
            background.push(tokio::spawn(store::guard(Arc::clone(&store), schedule)));
        }
        let observer = observing.map(|schedule| {
            let observer = Arc::new(Observer::new(
                Arc::clone(&supervisor),
                Arc::clone(&store),
                capture,
                paths.clone(),
                config.user.home.clone(),
            ));
            background.push(tokio::spawn(observer::observe(
                Arc::clone(&observer),
                schedule,
            )));
            observer
        });
        Ok(Self {
            supervisor,
            subprocess,
            store,
            applier,
            observer,
            paths,
            background,
        })
    }

    /// Serves `listener` until `shutdown` completes, then stops.
    ///
    /// Stopping follows S1: stop accepting connections, stop the tasks while open streams drain,
    /// because a stream ends only when its task stops, then clean up task processes.
    pub async fn serve(
        self,
        listener: UnixListener,
        shutdown: impl Future<Output = ()> + Send,
    ) -> Result<(), Error> {
        let own_uid = rustix::process::geteuid().as_raw();
        let tasks = GrpcApi::new(Arc::new(SupervisorApiAdapter::new(Arc::clone(
            &self.supervisor,
        ))))
        .with_authorizer(Arc::new(TaskAccess { own_uid }))
        .server();
        let owner = OwnerServer::new(OwnerService {
            store: Arc::clone(&self.store),
            applier: Arc::clone(&self.applier),
            observer: self.observer.clone(),
            paths: self.paths.clone(),
            supervisor: Arc::clone(&self.supervisor),
            own_uid,
        });

        let stop_serving = Arc::new(Notify::new());
        let mut server = tokio::spawn({
            let stop_serving = Arc::clone(&stop_serving);
            Server::builder()
                .add_service(InterceptedService::new(owner, PeerIdentity))
                .add_service(InterceptedService::new(tasks, PeerIdentity))
                .serve_with_incoming_shutdown(UnixListenerStream::new(listener), async move {
                    stop_serving.notified().await;
                })
        });

        let served = tokio::select! {
            () = shutdown => None,
            served = &mut server => Some(served),
        };
        stop_serving.notify_one();
        for task in &self.background {
            task.abort();
        }
        self.applier.stopping();
        let (drained, stopped) = tokio::join!(
            async {
                match served {
                    Some(served) => Some(served),
                    None => tokio::time::timeout(DRAIN, &mut server).await.ok(),
                }
            },
            self.supervisor.shutdown_with_timeout(STOP_TASKS),
        );
        if drained.is_none() {
            tracing::warn!("connections were still open after {DRAIN:?}; stopping anyway");
            server.abort();
        }
        if let Err(error) = self.subprocess.shutdown(STOP_PROCESSES).await {
            tracing::warn!(%error, "task processes were not cleaned up");
        }
        stopped?;
        match drained {
            Some(Ok(result)) => result.map_err(Error::Serve),
            Some(Err(stopped)) => Err(Error::Stopped(stopped.to_string())),
            None => Ok(()),
        }
    }
}

/// Rejects a configuration `lmxd` cannot work with, before anything starts.
fn check(config: &Config) -> Result<(), Error> {
    let tools = &config.tools;
    for (name, path) in [
        ("nix_store", &tools.nix_store),
        ("nice", &tools.nice),
        ("ionice", &tools.ionice),
        ("grep", &tools.grep),
        ("systemctl", &tools.systemctl),
        ("nixos_rebuild", &tools.nixos_rebuild),
        ("nix_env", &tools.nix_env),
        ("sudo", &tools.sudo),
        ("bash", &tools.bash),
        ("systemd_run", &tools.systemd_run),
        ("journalctl", &tools.journalctl),
    ] {
        if !Path::new(path).is_absolute() {
            return Err(Error::Config(format!(
                "tools.{name} must be an absolute path, not {path:?}"
            )));
        }
    }
    let disk = config.disk;
    if disk.minimum_percent > disk.collect_percent || disk.collect_percent > 100 {
        return Err(Error::Config(format!(
            "disk thresholds must satisfy minimum_percent <= collect_percent <= 100, not {} and {}",
            disk.minimum_percent, disk.collect_percent
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let tool = |name: &str| format!("/run/current-system/sw/bin/{name}");
        let json = serde_json::json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "0123456789ab",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": 100},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": tool("ip"),
                "systemctl": tool("systemctl"),
                "nix_store": tool("nix-store"),
                "nice": tool("nice"),
                "ionice": tool("ionice"),
                "grep": tool("grep"),
                "nixos_rebuild": tool("nixos-rebuild"),
                "nix_env": tool("nix-env"),
                "sudo": "/run/wrappers/bin/sudo",
                "bash": tool("bash"),
                "systemd_run": tool("systemd-run"),
                "journalctl": tool("journalctl")
            }
        });
        Config::from_json(json.to_string().as_bytes()).expect("valid configuration")
    }

    #[test]
    fn accepts_the_platform_configuration() {
        assert!(check(&config()).is_ok());
    }

    #[test]
    fn rejects_a_tool_without_an_absolute_path() {
        let mut config = config();
        config.tools.nix_store = "nix-store".into();
        let error = check(&config).expect_err("relative tool");
        assert!(error.to_string().contains("tools.nix_store"), "{error}");
    }

    #[test]
    fn rejects_a_minimum_above_the_collect_threshold() {
        let mut config = config();
        config.disk.minimum_percent = 30;
        assert!(check(&config).is_err());
    }
}
