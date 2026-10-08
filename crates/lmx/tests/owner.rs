//! `lmx` with a running `lmxd`: the owner part of `status`, `store reserve`, `apply` and the wait for
//! a converged generation.

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use lmx_model::{Config, DiskUsage};
use lmxd::{Daemon, GuardSchedule, ObserverSchedule, Options, UsageSource};
use serde_json::{Value, json};
use tokio::{net::UnixListener, runtime::Runtime, sync::oneshot, task::JoinHandle};

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

const NIXOS_REBUILD: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'nixos-rebuild %s\n' "$*" >> "$root/calls"
echo "building the system configuration..." >&2
echo "HTTP_PROXY=$HTTP_PROXY"
case $(/bin/cat "$root/build" 2>/dev/null) in
  fail) echo "error: builder for 'system.drv' failed" >&2; exit 1 ;;
  hang) exec /bin/sleep 60 ;;
esac
/bin/cp "$root/mnt/limanix/flake/runtime.json" "$root/nix/var/nix/profiles/system/etc/lmx/config.json"
"#;

/// Fake `nix-env`: logs its call; `--delete-generations old` removes the first generation link.
const NIX_ENV: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'nix-env %s\n' "$*" >> "$root/calls"
/bin/rm -f "$2-1-link"
"#;

const SWITCH_TO_CONFIGURATION: &str = r#"#!/bin/sh
root=${0%/nix/var/nix/profiles/system/bin/*}
printf 'switch-to-configuration %s\n' "$*" >> "$root/calls"
if [ "$(/bin/cat "$root/finalize" 2>/dev/null)" = fail ]; then
  echo 'boot loader update failed' >&2
  exit 1
fi
"#;

const SYSTEMCTL: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'systemctl %s\n' "$*" >> "$root/calls"
"#;

const SYSTEMD_RUN: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'systemd-run %s\n' "$*" >> "$root/calls"
while [ "${1#--}" != "$1" ]; do shift; done
exec "$@"
"#;

const SUDO: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'sudo %s %s %s\n' "$1" "$2" "$3" >> "$root/calls"
"#;

const MOUNTINFO: &str = "\
35 1 0:30 / /mnt/limanix ro,relatime - virtiofs mount0 ro
36 1 0:31 / /home/dev rw,relatime - virtiofs mount1 rw
";

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

fn usage(free_percent: u64) -> DiskUsage {
    DiskUsage {
        bytes: 1000 << 20,
        free_bytes: (free_percent * 10) << 20,
        available_bytes: (free_percent * 10) << 20,
        inodes: 1000,
        free_inodes: 500,
    }
}

struct Guest {
    root: tempfile::TempDir,
    runtime: Runtime,
    stop: Option<oneshot::Sender<()>>,
    served: Option<JoinHandle<Result<(), lmxd::Error>>>,
}

impl Guest {
    fn new(before: DiskUsage, after: DiskUsage) -> Self {
        Self::start(before, after, None, None)
    }

    fn observed() -> Self {
        let observer = ObserverSchedule {
            first: Duration::ZERO,
            every: Duration::from_millis(200),
            retry: Duration::from_secs(3600),
        };
        Self::start(usage(50), usage(50), None, Some(observer))
    }

    fn with_guard(before: DiskUsage, after: DiskUsage) -> Self {
        let guard = GuardSchedule {
            first: Duration::ZERO,
            every: Duration::from_secs(3600),
        };
        Self::start(before, after, Some(guard), None)
    }

    fn start(
        before: DiskUsage,
        after: DiskUsage,
        guard: Option<GuardSchedule>,
        observer: Option<ObserverSchedule>,
    ) -> Self {
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
            ("nixos-rebuild", NIXOS_REBUILD.to_owned()),
            ("nix-env", NIX_ENV.to_owned()),
            ("systemctl", SYSTEMCTL.to_owned()),
            ("sudo", SUDO.to_owned()),
            ("systemd-run", SYSTEMD_RUN.to_owned()),
        ] {
            executable(&path(&format!("bin/{tool}")), &script);
        }
        let gid = fs::metadata(root.path()).expect("tree metadata").gid();

        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "0123456789ab",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": gid},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": path("bin/ip"),
                "systemctl": path("bin/systemctl"),
                "nix_store": path("bin/nix-store"),
                "nice": path("bin/nice"),
                "ionice": path("bin/ionice"),
                "grep": "/usr/bin/grep",
                "nixos_rebuild": path("bin/nixos-rebuild"),
                "nix_env": path("bin/nix-env"),
                "sudo": path("bin/sudo"),
                "bash": path("bin/bash"),
                "systemd_run": path("bin/systemd-run"),
                "journalctl": path("bin/journalctl")
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
                observer,
                root: root.path().to_path_buf(),
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

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    fn mount(&self, desired: &str, built: &str, booted: &str, kept: usize) {
        let marker = |generation: &str| json!({"generation": generation}).to_string();
        self.write("mnt/limanix/flake/runtime.json", &marker(desired));
        self.write(
            "mnt/limanix/environment",
            "HTTP_PROXY=\"http://proxy:3128\"\n",
        );
        self.write(
            "mnt/limanix/environment.sh",
            "export HTTP_PROXY=\"http://proxy:3128\"\n",
        );
        self.write(
            "nix/var/nix/profiles/system/etc/lmx/config.json",
            &marker(built),
        );
        executable(
            &self.path("nix/var/nix/profiles/system/bin/switch-to-configuration"),
            SWITCH_TO_CONFIGURATION,
        );
        for number in 1..=kept {
            self.write(&format!("nix/var/nix/profiles/system-{number}-link"), "");
        }
        self.write("run/booted-system/etc/lmx/config.json", &marker(booted));
        self.write("proc/self/mountinfo", MOUNTINFO);
    }

    fn wait_for_call(&self, prefix: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.calls().iter().any(|call| call.starts_with(prefix)) {
            assert!(
                Instant::now() < deadline,
                "no {prefix} in {:?}",
                self.calls()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

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

fn executable(path: &Path, script: &str) {
    fs::create_dir_all(path.parent().expect("script has a parent")).expect("create parent");
    fs::write(path, script).expect("write a script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make it executable");
}

fn json_lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect()
}

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
    assert!(
        !guest.calls().iter().any(|call| call.contains("--gc")),
        "status collects nothing"
    );
}

#[test]
fn an_apply_installs_the_environment_and_builds_for_the_next_boot() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    let output = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .output()
        .expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let lines = json_lines(&output);
    let (envelope, events) = lines.split_last().expect("an answer");
    assert_eq!(
        envelope,
        &json!({"contract": 1, "ok": true, "data": {"generation": "g2", "state": "restart_required"}})
    );
    let phases: Vec<&Value> = events
        .iter()
        .filter(|event| event["event"] == "phase")
        .map(|event| &event["phase"])
        .collect();
    assert_eq!(phases, ["environment", "reserve", "build"]);
    for (stream, line) in [
        ("stderr", "building the system configuration..."),
        ("stdout", "HTTP_PROXY=http://proxy:3128"),
    ] {
        let event = json!({"event": "output", "stream": stream, "line": line});
        assert!(events.contains(&event), "{event} missing from {events:?}");
    }
    let environment = guest.path("etc/limanix/environment");
    assert_eq!(
        fs::read_to_string(&environment).expect("installed"),
        "HTTP_PROXY=\"http://proxy:3128\"\n"
    );
    assert_eq!(
        fs::metadata(&environment).expect("installed").mode() & 0o777,
        0o640
    );

    let again = guest
        .lmx(&["apply", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(answer(&again)["data"]["state"], "restart_required");
    let status = guest.lmx(&["status", "--json"]).output().expect("run lmx");
    assert_eq!(
        answer(&status)["data"]["owner"]["conditions"][0]["type"],
        "RestartRequired"
    );
    let other = guest
        .lmx(&["apply", "-g", "g3", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(other.status.code(), Some(1), "{other:?}");
    assert_eq!(
        answer(&other)["error"]["details"],
        json!({"requested": "g3", "mounted": "g2"})
    );
}

#[test]
fn a_failed_build_reports_its_exit_status() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    guest.write("build", "fail");
    let output = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .output()
        .expect("run lmx");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let lines = json_lines(&output);
    let envelope = lines.last().expect("an answer");
    assert_eq!(envelope["error"]["code"], "apply.build_failed");
    assert_eq!(envelope["error"]["details"], json!({"exit_code": 1}));
    assert!(lines.contains(&json!({
        "event": "output",
        "stream": "stderr",
        "line": "error: builder for 'system.drv' failed"
    })));
}

#[test]
fn a_cancel_stops_the_build_and_its_followers() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    guest.write("build", "hang");
    let follower = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .spawn()
        .expect("run lmx");
    guest.wait_for_call("nixos-rebuild");

    let cancel = guest
        .lmx(&["apply", "cancel", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert!(cancel.status.success(), "{cancel:?}");
    assert_eq!(answer(&cancel)["data"], json!({"cancelled": true}));

    let output = follower.wait_with_output().expect("wait for lmx");
    assert_eq!(output.status.code(), Some(130), "{output:?}");
    let lines = json_lines(&output);
    assert_eq!(
        lines.last().expect("an answer")["error"]["code"],
        "apply.cancelled"
    );
    let again = guest
        .lmx(&["apply", "cancel", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(answer(&again)["data"], json!({"cancelled": false}));
}

#[test]
fn a_healthy_booted_generation_is_finalized_and_converges() {
    let guest = Guest::observed();
    guest.mount("g1", "g1", "g1", 2);
    let output = guest
        .lmx(&[
            "status",
            "--wait",
            "converged",
            "-g",
            "g1",
            "--timeout",
            "60s",
            "--json",
        ])
        .output()
        .expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let conditions = &answer(&output)["data"]["owner"]["conditions"];
    assert_eq!(conditions[0]["type"], "Converged", "{conditions}");
    assert!(!guest.path("nix/var/nix/profiles/system-1-link").exists());
    let profile = guest
        .path("nix/var/nix/profiles/system")
        .display()
        .to_string();
    let calls = guest.calls();
    for call in [
        "systemctl is-active --quiet -- sshd.service".to_owned(),
        "sudo --set-home --user dev".to_owned(),
        format!("nix-env --profile {profile} --delete-generations old"),
        format!(
            "systemd-run --collect --no-ask-password --pipe --quiet --service-type=exec \
             --unit=nixos-rebuild-switch-to-configuration --wait {profile}/bin/switch-to-configuration boot"
        ),
        "switch-to-configuration boot".to_owned(),
    ] {
        assert!(calls.contains(&call), "{call} missing from {calls:?}");
    }
}

#[test]
fn a_failed_finalize_ends_the_wait_at_once() {
    let guest = Guest::observed();
    guest.write("finalize", "fail");
    guest.mount("g1", "g1", "g1", 2);
    let started = Instant::now();
    let output = guest
        .lmx(&[
            "status",
            "--wait",
            "converged",
            "-g",
            "g1",
            "--timeout",
            "60s",
            "--json",
        ])
        .output()
        .expect("run lmx");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the wait ended well before its timeout"
    );
    let error = &answer(&output)["error"];
    assert_eq!(error["code"], "finalize.failed", "{error}");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("boot loader update failed")),
        "{error}"
    );
    assert_eq!(
        error["details"]["conditions"][0]["type"], "FinalizeFailed",
        "{error}"
    );

    let status = guest.lmx(&["status", "--json"]).output().expect("run lmx");
    let conditions = &answer(&status)["data"]["owner"]["conditions"];
    assert_eq!(conditions[0]["type"], "FinalizeFailed", "{conditions}");
}

#[test]
fn doctor_and_the_short_status_follow_the_conditions_of_lmxd() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g2", "g1", 1);
    let output = guest.lmx(&["doctor", "--json"]).output().expect("run lmx");
    assert!(output.status.success(), "warnings do not fail: {output:?}");
    let checks = &answer(&output)["data"]["checks"];
    assert_eq!(checks[1]["status"], "ok", "{checks}");
    assert_eq!(
        checks[2],
        json!({
            "check": "generations",
            "status": "warning",
            "message": "Generation g2 is built; restart the VM to boot it.",
            "hint": "Restart the VM from the Mac; limanix update does it."
        })
    );

    let short = guest.lmx(&["status", "--short"]).output().expect("run lmx");
    assert!(short.status.success(), "{short:?}");
    assert_eq!(String::from_utf8_lossy(&short.stdout), "restart\n");
}
