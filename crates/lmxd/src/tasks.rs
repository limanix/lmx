//! Workload kinds of `lmxd` and the runner that executes them.
//!
//! Every operation of `lmxd` is a Solti task whose kind lives under [`API_VERSION`], so the Task API
//! lists it, streams its output and keeps its runs. Each kind runs one program. The runner turns
//! such a task into a `solti.io/v1` subprocess task and builds it with a private subprocess runner,
//! which clears the environment, owns the process group and captures the output. The private runner
//! is not registered with the supervisor, so no API caller can start an arbitrary program as root.

use std::sync::Arc;

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

/// API version of the workload kinds of `lmxd`.
pub(crate) const API_VERSION: &str = "lmx.limanix.dev/v1";

/// Shell script that lists garbage-collector roots, leaving out those that always exist: running
/// processes, runtime state and the system profile. `$1` is `nix-store` and `$2` is `grep`. Its exit
/// status is ignored, as the platform's store guard ignored it.
const ROOTS_SCRIPT: &str = r#""$1" --gc --print-roots | "$2" -E -v -e '^"?/proc/' -e '^"?/run/' -e '^"?/nix/var/nix/profiles/system' -e '[{]censored[}]'
exit 0"#;

/// Workload kinds of `lmxd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Collects unreferenced store paths with `nix-store --gc`.
    StoreCollect,
    /// Prints the garbage-collector roots that keep store paths alive.
    StoreRoots,
}

impl Kind {
    /// Every kind, in declaration order.
    const ALL: [Self; 2] = [Self::StoreCollect, Self::StoreRoots];

    /// Kind name under [`API_VERSION`].
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::StoreCollect => "StoreCollect",
            Self::StoreRoots => "StoreRoots",
        }
    }

    /// Prefix of the names of this kind's tasks; a counter follows it.
    pub(crate) const fn task_prefix(self) -> &'static str {
        match self {
            Self::StoreCollect => "store-collect",
            Self::StoreRoots => "store-roots",
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

/// Workload of `kind` with `spec`.
fn extension(kind: Kind, spec: Value) -> ModelResult<TaskWorkload> {
    ExtensionWorkload::new(API_VERSION, kind.name(), spec).map(TaskWorkload::Extension)
}

/// Program and arguments that run a task of `kind` with `spec`.
fn command(kind: Kind, spec: &Value, tools: &Tools) -> (String, Vec<String>) {
    match kind {
        Kind::StoreCollect => {
            let collect = [tools.nix_store.clone(), "--gc".into(), "--quiet".into()];
            if spec.get("priority").and_then(Value::as_str) == Some(Priority::Idle.as_str()) {
                let mut args = vec!["-n".into(), "19".into(), tools.ionice.clone()];
                args.extend(["-c".into(), "3".into()]);
                args.extend(collect);
                (tools.nice.clone(), args)
            } else {
                let [program, args @ ..] = collect;
                (program, args.to_vec())
            }
        }
        Kind::StoreRoots => (
            "/bin/sh".into(),
            vec![
                "-c".into(),
                ROOTS_SCRIPT.into(),
                "sh".into(),
                tools.nix_store.clone(),
                tools.grep.clone(),
            ],
        ),
    }
}

/// Runner of the `lmxd` kinds.
struct LmxRunner {
    /// Catalog of the private subprocess runner.
    catalog: RunnerCatalog,
    /// Absolute paths of the programs the kinds run.
    tools: Tools,
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
        let (command, args) = command(kind, workload.spec(), &self.tools);
        let process = TaskWorkload::Subprocess(SubprocessSpec::new(
            SubprocessMode::Command { command, args },
            TaskEnv::new(),
            None,
            Flag::enabled(),
        ));
        // Same name, generation and status, so output and runs belong to the `lmxd` task.
        let derived = Task::from_parts(
            task.type_meta().clone(),
            task.metadata().clone(),
            task.spec()
                .derive_with_workload(process)
                .without_runner_selector(),
            task.status().clone(),
        )
        .map_err(|error| RunnerError::InvalidSpec(error.to_string()))?;
        let built = self
            .catalog
            .build_scoped_with_cancellation(&derived, ctx, cancellation, scope)
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
    tools: Tools,
) -> Result<Arc<SubprocessRunner>, RegisterError> {
    let mut private = RunnerRouter::new();
    let subprocess = register_subprocess_runner(&mut private, "lmx-exec")?;
    router.register(Arc::new(LmxRunner {
        catalog: private.catalog(),
        tools,
    }))?;
    Ok(subprocess)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

    use super::*;

    /// Tools with recognizable paths.
    fn tools() -> Tools {
        Tools {
            ip: "/bin/ip".into(),
            systemctl: "/bin/systemctl".into(),
            nix_store: "/nix/bin/nix-store".into(),
            nice: "/bin/nice".into(),
            ionice: "/bin/ionice".into(),
            grep: "/bin/grep".into(),
        }
    }

    #[test]
    fn collects_at_normal_priority() {
        let (program, args) = command(Kind::StoreCollect, &json!({"priority": "normal"}), &tools());
        assert_eq!(program, "/nix/bin/nix-store");
        assert_eq!(args, ["--gc", "--quiet"]);
    }

    #[test]
    fn collects_for_the_guard_at_idle_priority() {
        let (program, args) = command(Kind::StoreCollect, &json!({"priority": "idle"}), &tools());
        assert_eq!(program, "/bin/nice");
        assert_eq!(
            args,
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
        let (program, args) = command(Kind::StoreRoots, &json!({}), &tools());
        assert_eq!(program, "/bin/sh");
        assert_eq!(args[..2], ["-c".to_owned(), ROOTS_SCRIPT.to_owned()]);
        assert_eq!(args[2..], ["sh", "/nix/bin/nix-store", "/bin/grep"]);
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
