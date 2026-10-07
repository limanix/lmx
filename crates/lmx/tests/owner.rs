//! `lmx` with a running `lmxd`: the owner part of `status`, and `store reserve`.
//!
//! Each test starts `lmxd` in this process on the socket of a prepared guest tree. The store disk is
//! read from a file, and a fake `nix-store` rewrites that file when it collects, so a test decides
//! what a collection frees.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use lmx_model::{Config, DiskUsage};
use lmxd::{Daemon, GuardSchedule, Options, UsageSource};
use serde_json::{Value, json};
use tokio::{net::UnixListener, runtime::Runtime, sync::oneshot, task::JoinHandle};

/// Fake `nix-store`. It logs its arguments next to the guest tree's `bin`. A collection takes two
/// seconds, so concurrent callers overlap, and then turns the disk into `disk-after.json`. A roots
/// listing prints a root the report leaves out and one it keeps.
const NIX_STORE: &str = r#"#!/bin/sh
root=$(dirname "$(dirname "$0")")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  "--gc --quiet") sleep 2; cp "$root/disk-after.json" "$root/disk.json" ;;
  "--gc --print-roots")
    echo '/proc/1/environ -> /nix/store/aaaa-glibc'
    echo '/home/dev/project/.direnv/flake-inputs/cccc-source -> /nix/store/cccc-source' ;;
esac
"#;

/// Fake `nice` or `ionice`: logs its name and arguments, drops its two options and runs the rest.
fn priority_tool(name: &str) -> String {
    format!(
        r#"#!/bin/sh
root=$(dirname "$(dirname "$0")")
printf '{name} %s\n' "$*" >> "$root/calls"
shift 2
exec "$@"
"#
    )
}

/// A 1000 MiB store disk with `free_percent` of its bytes free and plenty of inodes.
fn usage(free_percent: u64) -> DiskUsage {
    DiskUsage {
        bytes: 1000 << 20,
        free_bytes: (free_percent * 10) << 20,
        available_bytes: (free_percent * 10) << 20,
        inodes: 1000,
        free_inodes: 500,
    }
}

/// A guest tree with `lmxd` serving its socket.
struct Guest {
    /// Root replacing `/`; removed when the test ends.
    root: tempfile::TempDir,
    /// Runtime of the daemon.
    runtime: Runtime,
    /// Stops the daemon.
    stop: Option<oneshot::Sender<()>>,
    /// The serving daemon.
    served: Option<JoinHandle<Result<(), lmxd::Error>>>,
}

impl Guest {
    /// A guest whose store disk is `before` until a collection makes it `after`.
    fn new(before: DiskUsage, after: DiskUsage) -> Self {
        Self::start(before, after, None)
    }

    /// Like [`Guest::new`], with a store guard that checks the disk at once.
    fn with_guard(before: DiskUsage, after: DiskUsage) -> Self {
        let guard = GuardSchedule {
            first: Duration::ZERO,
            every: Duration::from_secs(3600),
        };
        Self::start(before, after, Some(guard))
    }

    /// Prepares the tree and starts `lmxd` with `guard`.
    fn start(before: DiskUsage, after: DiskUsage, guard: Option<GuardSchedule>) -> Self {
        let root = tempfile::tempdir().expect("temporary guest root");
        let path = |relative: &str| root.path().join(relative);
        fs::write(path("disk.json"), json!(before).to_string()).expect("write the disk");
        fs::write(path("disk-after.json"), json!(after).to_string()).expect("write the disk");
        fs::create_dir_all(path("bin")).expect("create bin");
        fs::create_dir_all(path("nix/store")).expect("create the store");
        fs::create_dir_all(path("run/lmx")).expect("create the socket directory");
        fs::create_dir_all(path("etc/lmx")).expect("create the configuration directory");
        for (tool, script) in [
            ("nix-store", NIX_STORE.to_owned()),
            ("nice", priority_tool("nice")),
            ("ionice", priority_tool("ionice")),
        ] {
            let tool = path(&format!("bin/{tool}"));
            fs::write(&tool, script).expect("write a tool");
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))
                .expect("make a tool executable");
        }

        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "0123456789ab",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": path("bin/ip"),
                "systemctl": path("bin/systemctl"),
                "nix_store": path("bin/nix-store"),
                "nice": path("bin/nice"),
                "ionice": path("bin/ionice"),
                "grep": "/usr/bin/grep"
            }
        });
        fs::write(path("etc/lmx/config.json"), config.to_string())
            .expect("write the configuration");
        let config = Config::from_json(config.to_string().as_bytes()).expect("valid configuration");

        let disk = path("disk.json");
        let usage: UsageSource = Arc::new(move || {
            let data = fs::read(&disk).map_err(|error| error.to_string())?;
            serde_json::from_slice(&data).map_err(|error| error.to_string())
        });
        let runtime = Runtime::new().expect("runtime");
        let daemon = runtime
            .block_on(Daemon::start(Options {
                config,
                usage,
                guard,
            }))
            .expect("lmxd starts");
        let listener = {
            let _runtime = runtime.enter();
            UnixListener::bind(path("run/lmx/lmx.sock")).expect("bind the socket")
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let served = runtime.spawn(daemon.serve(listener, async {
            let _ = stopped.await;
        }));
        Self {
            root,
            runtime,
            stop: Some(stop),
            served: Some(served),
        }
    }

    /// Prepares `lmx` with the tree as its system root.
    fn lmx(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lmx"));
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", self.path("etc/lmx/config.json"))
            .env("PATH", self.path("empty"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Path of `relative` below the root.
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    /// Arguments of every `nix-store` call so far.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.path("calls"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(served) = self.served.take() {
            let _ = self.runtime.block_on(served);
        }
    }
}

/// Parses standard output as one JSON answer, whatever the exit status.
fn answer(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "standard output is one JSON answer ({error}): {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn concurrent_reserves_share_one_collection() {
    let guest = Guest::new(usage(15), usage(30));
    let first = guest
        .lmx(&["store", "reserve", "--json"])
        .spawn()
        .expect("run lmx");
    let second = guest
        .lmx(&["store", "reserve", "--json"])
        .spawn()
        .expect("run lmx");

    for child in [first, second] {
        let output = child.wait_with_output().expect("wait for lmx");
        assert!(output.status.success(), "{output:?}");
        let answer = answer(&output);
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["data"]["collected"], true);
        assert_eq!(answer["data"]["freed_bytes"], 150 << 20);
        assert_eq!(answer["data"]["after"]["free_bytes"], 300 << 20);
    }
    assert_eq!(guest.calls(), ["--gc --quiet"]);
}

#[test]
fn a_disk_that_stays_low_is_reported_with_its_roots() {
    let guest = Guest::new(usage(5), usage(8));
    let output = guest
        .lmx(&["store", "reserve", "--json"])
        .output()
        .expect("run lmx");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer = answer(&output);
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["error"]["code"], "disk.low");
    assert_eq!(answer["error"]["details"]["freed_bytes"], 30 << 20);
    // The roots report runs for the journal after the answer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while guest.calls().len() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(guest.calls(), ["--gc --quiet", "--gc --print-roots"]);
}

#[test]
fn the_guard_collects_at_idle_priority() {
    let guest = Guest::with_guard(usage(15), usage(30));
    let deadline = Instant::now() + Duration::from_secs(10);
    while guest.calls().len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }

    let nix_store = guest.path("bin/nix-store").display().to_string();
    let ionice = guest.path("bin/ionice").display().to_string();
    assert_eq!(
        guest.calls(),
        [
            format!("nice -n 19 {ionice} -c 3 {nix_store} --gc --quiet"),
            format!("ionice -c 3 {nix_store} --gc --quiet"),
            "--gc --quiet".to_owned(),
        ]
    );
}

#[test]
fn status_includes_the_owner_when_lmxd_runs() {
    let guest = Guest::new(usage(5), usage(5));
    let output = guest.lmx(&["status", "--json"]).output().expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let owner = &answer(&output)["data"]["owner"];
    assert_eq!(owner["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(owner["conditions"][0]["type"], "DiskLow");
    assert_eq!(owner["operations"], json!([]));
    assert!(guest.calls().is_empty(), "status collects nothing");
}
