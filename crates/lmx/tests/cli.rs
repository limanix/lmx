//! Command-line contract of the `lmx` binary, run against a prepared guest tree.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};

/// A guest tree with generation markers, a store directory, fake tools and a configuration.
struct Guest {
    /// Root replacing `/`; removed when the test ends.
    root: tempfile::TempDir,
}

impl Guest {
    /// Prepares a guest that built generation `0123456789ab` and still runs `ba9876543210`.
    fn new() -> Self {
        let guest = Self {
            root: tempfile::tempdir().expect("temporary guest root"),
        };
        guest.write("nix/store/.keep", "");
        guest.write(
            "mnt/limanix/flake/runtime.json",
            r#"{"generation": "0123456789ab"}"#,
        );
        guest.write(
            "nix/var/nix/profiles/system/etc/lmx/config.json",
            r#"{"generation": "0123456789ab"}"#,
        );
        guest.write(
            "run/booted-system/etc/lmx/config.json",
            r#"{"generation": "ba9876543210"}"#,
        );
        let ip = guest.tool(
            "ip",
            r#"[{"ifname":"enp0s1","address":"52:55:55:aa:bb:cc","addr_info":[{"family":"inet","scope":"global","local":"192.0.2.10"}]}]"#,
        );
        let systemctl = guest.tool(
            "systemctl",
            "limanix-store-guard.service loaded failed failed Guard",
        );
        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "ba9876543210",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "session": {"command": null, "providers": []},
            "tools": {"ip": ip, "systemctl": systemctl}
        });
        guest.write("etc/lmx/config.json", &config.to_string());
        guest
    }

    /// Writes `content` to `relative` below the root.
    fn write(&self, relative: &str, content: &str) {
        let path = self.root.path().join(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    /// Creates an executable that prints `output` and returns its path.
    ///
    /// The script uses only shell built-ins because the tests run `lmx` with an empty `PATH`.
    fn tool(&self, name: &str, output: &str) -> PathBuf {
        assert!(!output.contains('\''), "tool output is single-quoted");
        let path = self.root.path().join("tools").join(name);
        self.write(
            &format!("tools/{name}"),
            &format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n"),
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make tool executable");
        path
    }

    /// Path of the configuration inside the tree.
    fn config(&self) -> PathBuf {
        self.root.path().join("etc/lmx/config.json")
    }

    /// Prepares `lmx` with the tree as its system root.
    fn command(&self, args: &[&str], config: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lmx"));
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", config)
            .env("PATH", self.root.path().join("empty"));
        command
    }

    /// Runs `lmx` with the tree as its system root.
    fn lmx(&self, args: &[&str], config: &Path) -> Output {
        self.command(args, config).output().expect("run lmx")
    }
}

/// Parses standard output as one JSON answer.
fn answer(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "lmx failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = output
        .stdout
        .strip_suffix(b"\n")
        .expect("the answer ends with a newline");
    assert!(!line.contains(&b'\n'), "the answer is a single line");
    serde_json::from_slice(line).expect("standard output is one JSON answer")
}

#[test]
fn status_json_answers_the_host_contract() {
    let guest = Guest::new();
    let answer = answer(&guest.lmx(&["status", "--json"], &guest.config()));

    assert_eq!(answer["contract"], 1);
    assert_eq!(answer["ok"], true);
    let data = &answer["data"];
    assert_eq!(
        data["generations"],
        json!({"desired": "0123456789ab", "built": "0123456789ab", "booted": "ba9876543210"})
    );
    assert!(
        data["disk"]["bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
    );
    assert_eq!(
        data["interfaces"],
        json!([{"name": "enp0s1", "mac": "52:55:55:aa:bb:cc", "ipv4": ["192.0.2.10"]}])
    );
    assert_eq!(data["failed_units"], json!(["limanix-store-guard.service"]));
    assert!(
        data.get("problems").is_none(),
        "every fact was read: {data}"
    );
}

#[test]
fn status_reports_missing_facts_without_failing() {
    let guest = Guest::new();
    let marker = guest.root.path().join("mnt/limanix/flake/runtime.json");
    fs::remove_file(&marker).expect("remove the desired marker");
    fs::create_dir(&marker).expect("a directory is a marker that cannot be read");
    let missing = guest.root.path().join("missing.json");
    let answer = answer(&guest.lmx(&["status", "--json"], &missing));

    let data = &answer["data"];
    assert_eq!(answer["ok"], true);
    assert_eq!(data["generations"]["desired"], Value::Null);
    assert!(
        data["disk"].is_object(),
        "disk does not depend on the configuration"
    );
    assert_eq!(data["interfaces"], Value::Null);
    assert_eq!(data["failed_units"], Value::Null);
    let facts: Vec<&str> = data["problems"]
        .as_array()
        .expect("problems are listed")
        .iter()
        .map(|problem| problem["fact"].as_str().expect("fact name"))
        .collect();
    assert_eq!(
        facts,
        ["config", "generations", "interfaces", "failed_units"]
    );
}

#[test]
fn status_text_is_for_people() {
    let guest = Guest::new();
    let output = guest.lmx(&["status"], &guest.config());
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(
        text.contains("built 0123456789ab, booted ba9876543210"),
        "{text}"
    );
    assert!(text.contains("Network       enp0s1 192.0.2.10"), "{text}");
    assert!(
        text.contains("Failed units  limanix-store-guard.service"),
        "{text}"
    );
}

#[test]
fn version_json_names_the_contract() {
    let guest = Guest::new();
    let answer = answer(&guest.lmx(&["version", "--json"], &guest.config()));
    assert_eq!(
        answer,
        json!({"contract": 1, "ok": true, "data": {"version": env!("CARGO_PKG_VERSION"), "contract": 1}})
    );
}

#[test]
fn unknown_commands_are_usage_errors() {
    let guest = Guest::new();
    let output = guest.lmx(&["frobnicate"], &guest.config());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: lmx"));
}

#[test]
fn no_command_prints_help() {
    let guest = Guest::new();
    let output = guest.lmx(&[], &guest.config());
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: lmx"));
}

#[test]
fn a_closed_standard_output_ends_quietly() {
    let guest = Guest::new();
    for args in [&["status"][..], &["status", "--json"], &["version"], &[]] {
        let (reader, writer) = io::pipe().expect("pipe");
        drop(reader);
        let output = guest
            .command(args, &guest.config())
            .stdout(writer)
            .output()
            .expect("run lmx");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(
            output.stderr.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
