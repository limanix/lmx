//! Workload kinds of `lmxd` and the runner that executes them.
//!
//! Every operation of `lmxd` is a Solti task whose kind lives under [`API_VERSION`], so the Task API
//! lists it, streams its output and keeps its runs. Each kind runs one program. The runner turns
//! such a task into a `solti.io/v1` subprocess task and builds it with a private subprocess runner,
//! which clears the environment, owns the process group and captures the output. The private runner
//! is not registered with the supervisor, so no API caller can start an arbitrary program as root.

use std::{fs, sync::Arc};

use lmx_model::Tools;
use serde_json::{Value, json};
use solti::{
    exec::{
        ExecError,
        subprocess::{SubprocessRunner, register_subprocess_runner},
    },
    model::{
        ExtensionWorkload, Flag, ModelResult, SubprocessMode, SubprocessSpec, Task, TaskEnv,
        TaskWorkload, WorkloadTypeMeta,
    },
    runner::{
        BuildCancellation, BuildContext, BuildScope, RouterError, RunId, Runner, RunnerCatalog,
        RunnerError, RunnerRouter, async_trait,
    },
    taskvisor::TaskRef,
};

use crate::{
    capture::{Capture, Tee},
    environment,
    paths::Paths,
};

/// API version of the workload kinds of `lmxd`.
pub(crate) const API_VERSION: &str = "lmx.limanix.dev/v1";

/// `PATH` of the system tasks: the tools of the running system, as the host's login shell had them.
const SYSTEM_PATH: &str = "/run/current-system/sw/bin";

/// Shell script that lists garbage-collector roots, leaving out those that always exist: running
/// processes, runtime state and the system profile. `$1` is `nix-store` and `$2` is `grep`. Its exit
/// status is ignored, as the platform's store guard ignored it.
const ROOTS_SCRIPT: &str = r#""$1" --gc --print-roots | "$2" -E -v -e '^"?/proc/' -e '^"?/run/' -e '^"?/nix/var/nix/profiles/system' -e '[{]censored[}]'
exit 0"#;

/// Shell script of the health check: every platform unit is active, and the development account
/// runs a command in its login shell, as the host checked a new generation. `$1` is `systemctl`,
/// `$2` `sudo`, `$3` `bash` and `$4` the account; the units follow. The last line names a failure.
const HEALTH_SCRIPT: &str = r#"systemctl=$1 sudo=$2 bash=$3 user=$4
shift 4
for unit in "$@"; do
  "$systemctl" is-active --quiet -- "$unit" || { echo "$unit is not active"; exit 1; }
done
"$sudo" --set-home --user "$user" -- "$bash" --login -c 'cd -- "$HOME" && exec "$@"' limanix-command true \
  || { echo "$user cannot run a command"; exit 1; }"#;

/// Shell script of finalize: remove the older generations of the system profile, then rewrite the
/// boot entries so they offer only what is kept. `$1` is `nix-env`, `$2` the profile and `$3`
/// `systemd-run`.
///
/// `switch-to-configuration` runs in its own unit, as `nixos-rebuild` runs it: stopping `lmxd` or
/// cancelling the task never interrupts a boot loader update, and the shared unit name keeps it from
/// running beside one that `nixos-rebuild` started.
const FINALIZE_SCRIPT: &str = r#""$1" --profile "$2" --delete-generations old &&
"$3" --collect --no-ask-password --pipe --quiet --service-type=exec \
  --unit=nixos-rebuild-switch-to-configuration --wait "$2/bin/switch-to-configuration" boot"#;

/// Workload kinds of `lmxd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Collects unreferenced store paths with `nix-store --gc`.
    StoreCollect,
    /// Prints the garbage-collector roots that keep store paths alive.
    StoreRoots,
    /// Builds the mounted generation for the next boot with `nixos-rebuild boot`.
    SystemApply,
    /// Checks that the booted generation works.
    SystemHealth,
    /// Removes the older generations of the system profile and rewrites the boot entries.
    SystemFinalize,
}

impl Kind {
    /// Every kind, in declaration order.
    const ALL: [Self; 5] = [
        Self::StoreCollect,
        Self::StoreRoots,
        Self::SystemApply,
        Self::SystemHealth,
        Self::SystemFinalize,
    ];

    /// Kind name under [`API_VERSION`].
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::StoreCollect => "StoreCollect",
            Self::StoreRoots => "StoreRoots",
            Self::SystemApply => "SystemApply",
            Self::SystemHealth => "SystemHealth",
            Self::SystemFinalize => "SystemFinalize",
        }
    }

    /// Prefix of the names of this kind's tasks; a counter follows it.
    pub(crate) const fn task_prefix(self) -> &'static str {
        match self {
            Self::StoreCollect => "store-collect",
            Self::StoreRoots => "store-roots",
            Self::SystemApply => "system-apply",
            Self::SystemHealth => "system-health",
            Self::SystemFinalize => "system-finalize",
        }
    }

    /// Kind with the name `name`.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// CPU and I/O priority of a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Priority {
    /// Normal priority; reserve uses it, because the host waits for the result.
    Normal,
    /// The lowest CPU priority and the idle I/O class, as the platform's store guard ran.
    Idle,
}

impl Priority {
    /// Spec value of the priority.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Idle => "idle",
        }
    }
}

/// Workload of a `StoreCollect` task.
pub(crate) fn collect(priority: Priority) -> ModelResult<TaskWorkload> {
    extension(Kind::StoreCollect, json!({"priority": priority.as_str()}))
}

/// Workload of a `StoreRoots` task.
pub(crate) fn roots() -> ModelResult<TaskWorkload> {
    extension(Kind::StoreRoots, json!({}))
}

/// Workload of a `SystemApply` task for `generation`.
pub(crate) fn apply(generation: &str) -> ModelResult<TaskWorkload> {
    extension(Kind::SystemApply, json!({"generation": generation}))
}

/// Workload of a `SystemHealth` task.
pub(crate) fn health() -> ModelResult<TaskWorkload> {
    extension(Kind::SystemHealth, json!({}))
}

/// Workload of a `SystemFinalize` task.
pub(crate) fn finalize() -> ModelResult<TaskWorkload> {
    extension(Kind::SystemFinalize, json!({}))
}

/// Workload of `kind` with `spec`.
fn extension(kind: Kind, spec: Value) -> ModelResult<TaskWorkload> {
    ExtensionWorkload::new(API_VERSION, kind.name(), spec).map(TaskWorkload::Extension)
}

/// What the runner needs to turn a task into a process.
#[derive(Clone, Debug)]
pub(crate) struct Setup {
    /// Absolute paths of the programs the kinds run.
    pub(crate) tools: Tools,
    /// Guest locations.
    pub(crate) paths: Paths,
    /// Development account, which the health check runs a command as.
    pub(crate) user: String,
    /// Platform units the health check requires.
    pub(crate) units: Vec<String>,
}

/// A program to run, with its arguments and environment.
#[derive(Debug, PartialEq, Eq)]
struct Process {
    /// Absolute path of the program.
    program: String,
    /// Its arguments.
    args: Vec<String>,
    /// Its environment; the subprocess runner clears everything else.
    env: Vec<(String, String)>,
}

impl Process {
    /// `program` with `args` and an empty environment.
    fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            env: Vec::new(),
        }
    }
}

/// The process that runs a task of `kind` with `spec`.
fn process(kind: Kind, spec: &Value, setup: &Setup) -> Process {
    let tools = &setup.tools;
    match kind {
        Kind::StoreCollect => {
            let collect = [tools.nix_store.clone(), "--gc".into(), "--quiet".into()];
            if spec.get("priority").and_then(Value::as_str) == Some(Priority::Idle.as_str()) {
                let mut args = vec!["-n".into(), "19".into(), tools.ionice.clone()];
                args.extend(["-c".into(), "3".into()]);
                args.extend(collect);
                Process::new(tools.nice.clone(), args)
            } else {
                let [program, args @ ..] = collect;
                Process::new(program, args.to_vec())
            }
        }
        Kind::StoreRoots => Process::new(
            "/bin/sh",
            vec![
                "-c".into(),
                ROOTS_SCRIPT.into(),
                "sh".into(),
                tools.nix_store.clone(),
                tools.grep.clone(),
            ],
        ),
        Kind::SystemApply => {
            let mut process = Process::new(
                tools.nixos_rebuild.clone(),
                vec![
                    "boot".into(),
                    "--flake".into(),
                    setup.paths.flake(),
                    "--no-write-lock-file".into(),
                    "--no-update-lock-file".into(),
                ],
            );
            // The user's variables come last, so they win, as with systemd's `EnvironmentFile`.
            process.env.push(("PATH".into(), SYSTEM_PATH.into()));
            if let Ok(text) = fs::read_to_string(setup.paths.environment().join("environment")) {
                process.env.extend(environment::variables(&text));
            }
            process
        }
        Kind::SystemHealth => {
            let mut args = vec![
                "-c".into(),
                HEALTH_SCRIPT.into(),
                "sh".into(),
                tools.systemctl.clone(),
                tools.sudo.clone(),
                tools.bash.clone(),
                setup.user.clone(),
            ];
            args.extend(setup.units.iter().cloned());
            system(Process::new("/bin/sh", args))
        }
        Kind::SystemFinalize => system(Process::new(
            "/bin/sh",
            vec![
                "-c".into(),
                FINALIZE_SCRIPT.into(),
                "sh".into(),
                tools.nix_env.clone(),
                setup.paths.system_profile().display().to_string(),
                tools.systemd_run.clone(),
            ],
        )),
    }
}

/// `process` with the system tools in its `PATH`, which `switch-to-configuration` and the account's
/// login need.
fn system(mut process: Process) -> Process {
    process.env.push(("PATH".into(), SYSTEM_PATH.into()));
    process
}

/// Runner of the `lmxd` kinds.
struct LmxRunner {
    /// Catalog of the private subprocess runner.
    catalog: RunnerCatalog,
    /// What turns a task into a process.
    setup: Setup,
    /// Listeners of task output.
    capture: Arc<Capture>,
}

#[async_trait]
impl Runner for LmxRunner {
    fn name(&self) -> &str {
        "lmx"
    }

    fn workload_types(&self) -> Vec<WorkloadTypeMeta> {
        Kind::ALL
            .into_iter()
            .map(|kind| WorkloadTypeMeta::new(API_VERSION, kind.name()).expect("valid kind"))
            .collect()
    }

    async fn build_task(
        &self,
        task: &Task,
        _run_id: &RunId,
        ctx: &BuildContext,
        cancellation: &BuildCancellation,
        scope: &mut BuildScope,
    ) -> Result<TaskRef, RunnerError> {
        let TaskWorkload::Extension(workload) = task.spec().workload() else {
            return Err(RunnerError::InvalidSpec("not an lmxd workload".into()));
        };
        let kind = Kind::from_name(workload.kind())
            .ok_or_else(|| RunnerError::InvalidSpec(format!("unknown kind {}", workload.kind())))?;
        let Process { program, args, env } = process(kind, workload.spec(), &self.setup);
        let mut task_env = TaskEnv::new();
        for (name, value) in env {
            task_env.push(name, value);
        }
        let subprocess = TaskWorkload::Subprocess(SubprocessSpec::new(
            SubprocessMode::Command {
                command: program,
                args,
            },
            task_env,
            None,
            Flag::enabled(),
        ));
        // Same name, generation and status, so output and runs belong to the `lmxd` task.
        let derived = Task::from_parts(
            task.type_meta().clone(),
            task.metadata().clone(),
            task.spec()
                .derive_with_workload(subprocess)
                .without_runner_selector(),
            task.status().clone(),
        )
        .map_err(|error| RunnerError::InvalidSpec(error.to_string()))?;
        let tee = Arc::new(Tee {
            inner: Arc::clone(ctx.output_publisher()),
            capture: Arc::clone(&self.capture),
        });
        let built = self
            .catalog
            .build_scoped_with_cancellation(
                &derived,
                &ctx.clone().with_output_publisher(tee),
                cancellation,
                scope,
            )
            .await
            .map_err(|source| RunnerError::NestedBuild {
                context: format!("{} task {}", kind.name(), task.name()),
                source: Box::new(source),
            })?;
        Ok(built.into_task())
    }
}

/// Failure to register the runners.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// The subprocess runner could not be created.
    #[error("cannot create the subprocess runner: {0}")]
    Subprocess(#[from] ExecError),
    /// A runner was rejected by its router.
    #[error("cannot register a runner: {0}")]
    Router(#[from] RouterError),
}

/// Registers the runner of the `lmxd` kinds in `router`.
///
/// Returns the private subprocess runner, which must be shut down after the supervisor.
pub(crate) fn register(
    router: &mut RunnerRouter,
    setup: Setup,
    capture: Arc<Capture>,
) -> Result<Arc<SubprocessRunner>, RegisterError> {
    let mut private = RunnerRouter::new();
    let subprocess = register_subprocess_runner(&mut private, "lmx-exec")?;
    router.register(Arc::new(LmxRunner {
        catalog: private.catalog(),
        setup,
        capture,
    }))?;
    Ok(subprocess)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

    use super::*;

    /// A setup with recognizable tool paths below `root`.
    fn setup(root: &Path) -> Setup {
        Setup {
            tools: Tools {
                ip: "/bin/ip".into(),
                systemctl: "/bin/systemctl".into(),
                nix_store: "/nix/bin/nix-store".into(),
                nice: "/bin/nice".into(),
                ionice: "/bin/ionice".into(),
                grep: "/bin/grep".into(),
                nixos_rebuild: "/bin/nixos-rebuild".into(),
                nix_env: "/bin/nix-env".into(),
                sudo: "/bin/sudo".into(),
                bash: "/bin/bash".into(),
                systemd_run: "/bin/systemd-run".into(),
            },
            paths: Paths::new(root.to_path_buf()),
            user: "dev".into(),
            units: vec!["sshd.service".into(), "lmx.socket".into()],
        }
    }

    #[test]
    fn collects_at_normal_priority() {
        let process = process(
            Kind::StoreCollect,
            &json!({"priority": "normal"}),
            &setup(Path::new("/")),
        );
        assert_eq!(process.program, "/nix/bin/nix-store");
        assert_eq!(process.args, ["--gc", "--quiet"]);
    }

    #[test]
    fn collects_for_the_guard_at_idle_priority() {
        let process = process(
            Kind::StoreCollect,
            &json!({"priority": "idle"}),
            &setup(Path::new("/")),
        );
        assert_eq!(process.program, "/bin/nice");
        assert_eq!(
            process.args,
            [
                "-n",
                "19",
                "/bin/ionice",
                "-c",
                "3",
                "/nix/bin/nix-store",
                "--gc",
                "--quiet"
            ]
        );
    }

    #[test]
    fn lists_roots_through_the_configured_tools() {
        let process = process(Kind::StoreRoots, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.program, "/bin/sh");
        assert_eq!(
            process.args[..2],
            ["-c".to_owned(), ROOTS_SCRIPT.to_owned()]
        );
        assert_eq!(process.args[2..], ["sh", "/nix/bin/nix-store", "/bin/grep"]);
    }

    #[test]
    fn builds_the_mounted_flake_with_the_users_environment() {
        let root = tempfile::tempdir().expect("temporary root");
        let setup = setup(root.path());
        fs::create_dir_all(setup.paths.environment()).expect("create /etc/limanix");
        fs::write(
            setup.paths.environment().join("environment"),
            "HTTP_PROXY=\"http://proxy:3128\"\nPATH=\"/opt/bin\"\n",
        )
        .expect("write the environment");

        let process = process(Kind::SystemApply, &json!({"generation": "g1"}), &setup);
        assert_eq!(process.program, "/bin/nixos-rebuild");
        let flake = format!("path:{}/mnt/limanix/flake#runtime", root.path().display());
        assert_eq!(
            process.args,
            [
                "boot",
                "--flake",
                flake.as_str(),
                "--no-write-lock-file",
                "--no-update-lock-file"
            ]
        );
        assert_eq!(
            process.env,
            [
                ("PATH".to_owned(), SYSTEM_PATH.to_owned()),
                ("HTTP_PROXY".to_owned(), "http://proxy:3128".to_owned()),
                ("PATH".to_owned(), "/opt/bin".to_owned()),
            ]
        );
    }

    #[test]
    fn checks_health_with_the_units_and_the_account() {
        let process = process(Kind::SystemHealth, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.program, "/bin/sh");
        assert_eq!(
            process.args[2..],
            [
                "sh",
                "/bin/systemctl",
                "/bin/sudo",
                "/bin/bash",
                "dev",
                "sshd.service",
                "lmx.socket"
            ]
        );
    }

    #[test]
    fn finalizes_the_system_profile() {
        let process = process(Kind::SystemFinalize, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.args[1], FINALIZE_SCRIPT);
        assert_eq!(
            process.args[2..],
            [
                "sh",
                "/bin/nix-env",
                "/nix/var/nix/profiles/system",
                "/bin/systemd-run"
            ]
        );
        assert_eq!(process.env, [("PATH".to_owned(), SYSTEM_PATH.to_owned())]);
    }

    #[test]
    fn the_roots_report_leaves_out_the_roots_that_always_exist() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nix_store = directory.path().join("nix-store");
        fs::write(
            &nix_store,
            r#"#!/bin/sh
printf '%s\n' \
  '"/proc/1/maps" -> /nix/store/aaaa-glibc' \
  '/proc/2/environ -> /nix/store/bbbb-bash' \
  '/run/booted-system -> /nix/store/cccc-system' \
  '/nix/var/nix/profiles/system-42-link -> /nix/store/dddd-system' \
  '{censored} -> /nix/store/eeee-secret' \
  '/home/dev/project/.direnv/flake-inputs/ffff-source -> /nix/store/ffff-source'
"#,
        )
        .expect("write nix-store");
        fs::set_permissions(&nix_store, fs::Permissions::from_mode(0o755))
            .expect("make nix-store executable");
        let grep = ["/usr/bin/grep", "/bin/grep"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .expect("grep is installed");

        let output = Command::new("/bin/sh")
            .args(["-c", ROOTS_SCRIPT, "sh"])
            .arg(&nix_store)
            .arg(grep)
            .output()
            .expect("run the roots report");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "/home/dev/project/.direnv/flake-inputs/ffff-source -> /nix/store/ffff-source\n"
        );
    }

    #[test]
    fn names_every_kind_under_the_lmx_api_version() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            assert!(WorkloadTypeMeta::new(API_VERSION, kind.name()).is_ok());
        }
    }
}
