//! Command-line contract of the `lmx` binary, run against a prepared guest tree.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{self, Command, Output, Stdio},
    sync::OnceLock,
};

use serde_json::{Value, json};

struct Tools {
    ip: PathBuf,
    systemctl: PathBuf,
    journalctl: PathBuf,
}

fn tools() -> &'static Tools {
    static TOOLS: OnceLock<Tools> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fake-tools");
        Tools {
            ip: fake_tool(
                &directory,
                "ip",
                r#"[{"ifname":"enp0s1","address":"52:55:55:aa:bb:cc","addr_info":[{"family":"inet","scope":"global","local":"192.0.2.10"}]}]"#,
            ),
            systemctl: fake_tool(
                &directory,
                "systemctl",
                "limanix-store-guard.service loaded failed failed Guard",
            ),
            journalctl: fake_script(
                &directory,
                "journalctl",
                &format!(
                    "#!/bin/sh\n\
                     case \"$*\" in\n\
                     \x20 '') ;;\n\
                     \x20 *'-b0 _UID=0 LMX_KIND=SystemApply') printf '%s\\n' '{JOURNAL}' ;;\n\
                     \x20 *) echo 'No journal files were opened due to insufficient permissions.' >&2; exit 1 ;;\n\
                     esac\n"
                ),
            ),
        }
    })
}

fn fake_tool(directory: &Path, name: &str, output: &str) -> PathBuf {
    assert!(!output.contains('\''), "tool output is single-quoted");
    fake_script(
        directory,
        name,
        &format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n"),
    )
}

fn fake_script(directory: &Path, name: &str, script: &str) -> PathBuf {
    let path = directory.join(name);
    let current = fs::read_to_string(&path).ok().as_deref() == Some(script)
        && fs::metadata(&path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    if !current {
        let staged = directory.join(format!(".{name}.{}", process::id()));
        fs::create_dir_all(directory).expect("create the tool directory");
        fs::write(&staged, script).expect("write the tool");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
            .expect("make the tool executable");
        fs::rename(&staged, &path).expect("install the tool");
    }
    let first_run = Command::new(&path).output().expect("run the tool");
    assert!(first_run.status.success(), "{name} runs");
    path
}

const JOURNAL: &str = concat!(
    r#"{"MESSAGE":"applying generation 0123456789aa","LMX_TASK":"system-apply-1","LMX_GENERATION":"0123456789aa","_PID":"812","__REALTIME_TIMESTAMP":"1791364000000000"}"#,
    "\n",
    r#"{"MESSAGE":"applying generation 0123456789ab","LMX_TASK":"system-apply-2","LMX_GENERATION":"0123456789ab","_PID":"812","__REALTIME_TIMESTAMP":"1791364323000000"}"#,
    "\n",
    r#"{"MESSAGE":"building the system configuration...","LMX_TASK":"system-apply-2","_PID":"812","__REALTIME_TIMESTAMP":"1791364324000000"}"#,
);

const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0 100 0 0 10 0
";

const MOUNTINFO: &str = "\
22 1 254:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw
40 22 0:38 / /home/dev rw,relatime shared:20 - virtiofs mount0 rw
41 22 0:39 / /mnt/limanix ro,relatime shared:21 - virtiofs mount1 ro
";

struct Guest {
    root: tempfile::TempDir,
}

impl Guest {
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
        guest.write("proc/self/mountinfo", MOUNTINFO);
        guest.write(
            "proc/meminfo",
            "MemTotal:        7969124 kB\nMemFree:         5123456 kB\n",
        );
        let tools = tools();
        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "ba9876543210",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": 100},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": tools.ip,
                "systemctl": tools.systemctl,
                "nix_store": "/run/current-system/sw/bin/nix-store",
                "nice": "/run/current-system/sw/bin/nice",
                "ionice": "/run/current-system/sw/bin/ionice",
                "grep": "/run/current-system/sw/bin/grep",
                "nixos_rebuild": "/run/current-system/sw/bin/nixos-rebuild",
                "nix_env": "/run/current-system/sw/bin/nix-env",
                "sudo": "/run/wrappers/bin/sudo",
                "bash": "/run/current-system/sw/bin/bash",
                "systemd_run": "/run/current-system/sw/bin/systemd-run",
                "journalctl": tools.journalctl
            }
        });
        guest.write("etc/lmx/config.json", &config.to_string());
        guest
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.root.path().join(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    fn script(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.root.path().join(relative);
        self.write(relative, &format!("#!/bin/sh\n{body}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make script executable");
        path
    }

    fn set_session(&self, session: Value) {
        let mut config: Value =
            serde_json::from_slice(&fs::read(self.config()).expect("read configuration"))
                .expect("configuration JSON");
        config["session"] = session;
        self.write("etc/lmx/config.json", &config.to_string());
    }

    fn config(&self) -> PathBuf {
        self.root.path().join("etc/lmx/config.json")
    }

    fn command(&self, args: &[&str], config: &Path) -> Command {
        self.command_as(env!("CARGO_BIN_EXE_lmx"), args, config)
    }

    fn alias(&self, name: &str, args: &[&str]) -> Command {
        let link = self.root.path().join("aliases").join(name);
        if !link.exists() {
            fs::create_dir_all(link.parent().expect("link has a parent")).expect("create aliases");
            symlink(env!("CARGO_BIN_EXE_lmx"), &link).expect("link the binary");
        }
        self.command_as(link, args, &self.config())
    }

    fn command_as(&self, program: impl AsRef<Path>, args: &[&str], config: &Path) -> Command {
        let mut command = Command::new(program.as_ref());
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", config)
            .env("PATH", self.root.path().join("empty"))
            .env("LMX_TTY_IN", self.path("no-terminal"))
            .env("LMX_TTY_OUT", self.path("no-terminal"))
            .env_remove("TMUX")
            .env_remove("LMX_PASTE_TIMEOUT");
        command
    }

    fn lmx(&self, args: &[&str], config: &Path) -> Output {
        self.command(args, config).output().expect("run lmx")
    }
}

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

fn fake_tmux(guest: &Guest, answers: bool) -> PathBuf {
    let state = guest.path("tmux");
    fs::create_dir_all(&state).expect("create the tmux state");
    let state = state.display();
    let refresh = if answers {
        format!(": > {state}/refreshed")
    } else {
        ":".to_owned()
    };
    guest.script(
        "bin/tmux",
        &format!(
            r#"printf '%s\n' "$*" >> {state}/calls
case "$1" in
  load-buffer) while IFS= read -r line || [ -n "$line" ]; do printf '%s' "$line"; done > {state}/buffer ;;
  list-buffers) if [ -e {state}/refreshed ]; then echo '2 buffer1'; else echo '1 buffer0'; fi ;;
  refresh-client) {refresh} ;;
  save-buffer) printf '%s' pasted ;;
esac
"#
        ),
    );
    guest.path("bin")
}

fn in_tmux<'a>(command: &'a mut Command, bin: &Path) -> &'a mut Command {
    command
        .env("TMUX", "/tmp/tmux-501/default,1,0")
        .env("PATH", bin)
}

fn run_with_input(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the command");
    child
        .stdin
        .take()
        .expect("standard input")
        .write_all(input)
        .expect("write standard input");
    child.wait_with_output().expect("wait for the command")
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
    // No lmxd runs in this guest: the owner is the only fact that cannot be read.
    assert_eq!(data["owner"], Value::Null);
    assert_eq!(data["problems"].as_array().map(Vec::len), Some(1), "{data}");
    assert_eq!(data["problems"][0]["fact"], "owner");
    assert!(
        data["problems"][0]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("lmxd is not reachable at ")),
        "{data}"
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
        [
            "config",
            "generations",
            "interfaces",
            "failed_units",
            "owner"
        ]
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
fn status_names_an_unreadable_disk() {
    let guest = Guest::new();
    fs::remove_dir_all(guest.path("nix/store")).expect("remove the store");
    let answer = answer(&guest.lmx(&["status", "--json"], &guest.config()));
    let problems = answer["data"]["problems"]
        .as_array()
        .expect("problems are listed");
    assert_eq!(answer["data"]["disk"], Value::Null);
    let facts: Vec<&Value> = problems.iter().map(|problem| &problem["fact"]).collect();
    assert_eq!(facts, ["disk", "owner"], "{problems:?}");
}

#[test]
fn owner_operations_without_lmxd_report_an_unavailable_owner() {
    let guest = Guest::new();
    for args in [
        &["store", "reserve", "--json"][..],
        &["apply", "-g", "0123456789ab", "--json"],
        &["apply", "cancel", "-g", "0123456789ab", "--json"],
        &[
            "status",
            "--wait",
            "converged",
            "-g",
            "0123456789ab",
            "--timeout",
            "1s",
            "--json",
        ],
    ] {
        let output = guest.lmx(args, &guest.config());
        assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
        let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
        assert_eq!(answer["ok"], false, "{args:?}");
        assert_eq!(answer["error"]["code"], "owner.unavailable", "{args:?}");
    }
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
fn every_way_of_asking_for_help_prints_the_guest_page() {
    let guest = Guest::new();
    for args in [
        &[][..],
        &["help"],
        &["--help"],
        &["-h"],
        &["help", "--help"],
        &["help", "-h"],
    ] {
        let output = guest.lmx(args, &guest.config());
        assert!(output.status.success(), "{args:?}");
        let text = String::from_utf8(output.stdout).expect("UTF-8 text");
        assert!(
            text.starts_with("LimaNix workspace\n\nVM: dev-box (arm64)\nUser: dev\n"),
            "{args:?}: {text}"
        );
        assert!(text.contains("  lmx info "), "{text}");
    }
}

#[test]
fn help_without_metadata_lists_commands_and_fails() {
    let guest = Guest::new();
    let output = guest.lmx(&["help"], &guest.path("missing.json"));
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(
        text.contains("Workspace metadata cannot be read: cannot read"),
        "{text}"
    );
    assert!(text.contains("  pbpaste "), "{text}");
}

#[test]
fn help_takes_no_other_arguments() {
    let guest = Guest::new();
    for args in [
        &["--help", "extra"][..],
        &["-h", "status"],
        &["help", "extra"],
    ] {
        let output = guest.lmx(args, &guest.config());
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn command_help_stays_generated() {
    let guest = Guest::new();
    let output = guest.lmx(&["status", "--help"], &guest.config());
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: lmx status"));
}

#[test]
fn info_shows_the_workspace_and_the_guest() {
    let guest = Guest::new();
    let output = guest.lmx(&["info"], &guest.config());
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(output.status.success(), "{text}");
    assert!(text.starts_with("VM: dev-box (arm64)\n"), "{text}");
    assert!(text.contains("\nKernel        "), "{text}");
    assert!(text.contains("\nDisk          "), "{text}");
    assert!(
        text.contains(
            "\nShared        /home/dev     virtiofs  rw\n              /mnt/limanix  virtiofs  ro\n"
        ),
        "{text}"
    );
    assert!(
        text.ends_with("Failed units  limanix-store-guard.service\n"),
        "{text}"
    );
}

#[test]
fn info_without_metadata_still_shows_the_guest() {
    let guest = Guest::new();
    let output = guest.lmx(&["info"], &guest.path("missing.json"));
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(
        text.starts_with("Workspace metadata cannot be read: cannot read"),
        "{text}"
    );
    assert!(text.contains("\nShared        /home/dev "), "{text}");
}

#[test]
fn info_fails_when_a_part_cannot_be_read() {
    let guest = Guest::new();
    fs::remove_file(guest.path("proc/self/mountinfo")).expect("remove the mount table");
    let output = guest.lmx(&["info"], &guest.config());
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(
        text.contains("Shared        unavailable: cannot read the mount table"),
        "{text}"
    );
    assert!(
        text.contains("Failed units  limanix-store-guard.service"),
        "{text}"
    );
}

#[test]
fn welcome_summarizes_the_vm() {
    let guest = Guest::new();
    let output = guest.lmx(&["welcome"], &guest.config());
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(output.status.success(), "{text}");
    assert!(!text.contains('\x1b'), "a pipe gets no colors: {text:?}");
    assert!(
        text.contains("  VM         dev-box (NixOS 26.05, arm64)\n"),
        "{text}"
    );
    assert!(text.contains(", 7.6 GiB memory, "), "{text}");
    assert!(
        text.contains("  Shared     /home/dev     rw\n             /mnt/limanix  ro\n"),
        "{text}"
    );
    assert!(
        text.contains("  ▲ Failed: limanix-store-guard.service. Run lmx info for details.\n"),
        "{text}"
    );
}

#[test]
fn welcome_without_metadata_still_greets() {
    let guest = Guest::new();
    let output = guest.lmx(&["welcome"], &guest.path("missing.json"));
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("  VM         unknown (workspace metadata cannot be read)\n")
    );
}

#[test]
fn welcome_fails_without_the_mount_table() {
    let guest = Guest::new();
    fs::remove_file(guest.path("proc/self/mountinfo")).expect("remove the mount table");
    let output = guest.lmx(&["welcome"], &guest.config());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("  Shared     unavailable; run lmx info\n")
    );
}

#[test]
fn session_passes_one_name_to_the_provider_unchanged() {
    let guest = Guest::new();
    let provider = guest.script("tools/provider", "printf '%s\\n' \"$@\"\nexit 7\n");
    guest.set_session(json!({"command": provider, "providers": []}));
    for name in ["--help", "my project"] {
        let output = guest
            .alias("limanix-session", &[name])
            .output()
            .expect("run limanix-session");
        assert_eq!(output.status.code(), Some(7), "the provider's status");
        assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{name}\n"));
    }
    let output = guest.lmx(&["session", "demo"], &guest.config());
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "demo\n");
}

#[test]
fn session_without_a_provider_suggests_one() {
    let guest = Guest::new();
    for command in [Value::Null, json!("")] {
        guest.set_session(json!({"command": command, "providers": ["lmx:tmux"]}));
        let output = guest
            .alias("limanix-session", &["demo"])
            .output()
            .expect("run limanix-session");
        assert_eq!(output.status.code(), Some(127), "{command}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "No session provider is configured in this VM.\n\
             Select a session provider in nixos.modules: lmx:tmux\n\
             Apply the configuration with limanix update --config <file> before reconnecting.\n"
        );
    }
}

#[test]
fn session_reports_a_provider_that_cannot_start() {
    let guest = Guest::new();
    guest.set_session(json!({"command": guest.path("missing/provider"), "providers": []}));
    let output = guest
        .alias("limanix-session", &["demo"])
        .output()
        .expect("run limanix-session");
    assert_eq!(output.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("lmx: cannot run "));

    let output = guest
        .alias("limanix-session", &["demo"])
        .env("LMX_CONFIG", guest.path("missing.json"))
        .output()
        .expect("run limanix-session");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("lmx: cannot read "));
}

#[test]
fn session_names_are_one_nonempty_argument() {
    let guest = Guest::new();
    for args in [&[][..], &[""], &["a", "b"]] {
        let output = guest
            .alias("limanix-session", args)
            .output()
            .expect("run limanix-session");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "Usage: limanix-session NAME (one nonempty session name)\n"
        );
    }
}

#[test]
fn clipboard_aliases_take_no_arguments() {
    let guest = Guest::new();
    for (name, usage) in [
        ("pbcopy", "Usage: pbcopy < FILE\n"),
        ("pbpaste", "Usage: pbpaste\n"),
    ] {
        let output = guest
            .alias(name, &["extra"])
            .output()
            .expect("run the alias");
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert_eq!(String::from_utf8_lossy(&output.stderr), usage);
    }
}

#[test]
fn pbcopy_writes_the_clipboard_to_the_terminal() {
    let guest = Guest::new();
    guest.write("tty", "");
    let output = run_with_input(
        guest
            .alias("pbcopy", &[])
            .env("LMX_TTY_OUT", guest.path("tty")),
        b"hello\n",
    );
    assert!(output.status.success());
    assert_eq!(
        fs::read(guest.path("tty")).expect("read the terminal"),
        b"\x1b]52;c;aGVsbG8K\x07"
    );
}

#[test]
fn pbcopy_needs_a_terminal() {
    let guest = Guest::new();
    let output = run_with_input(
        guest
            .alias("pbcopy", &[])
            .env("LMX_TTY_OUT", guest.path("missing/tty")),
        b"hello",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbcopy needs the terminal of a limanix shell session.\n"
    );
}

#[test]
fn pbpaste_prints_the_terminal_reply() {
    let guest = Guest::new();
    guest.write("reply", "\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
    guest.write("request", "");
    let output = guest
        .alias("pbpaste", &[])
        .env("LMX_TTY_IN", guest.path("reply"))
        .env("LMX_TTY_OUT", guest.path("request"))
        .output()
        .expect("run pbpaste");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello world");
    assert_eq!(
        fs::read(guest.path("request")).expect("read the request"),
        b"\x1b]52;c;?\x07"
    );
}

#[test]
fn pbpaste_explains_an_unanswered_request() {
    let guest = Guest::new();
    guest.write("reply", "");
    guest.write("request", "");
    let output = guest
        .alias("pbpaste", &[])
        .env("LMX_TTY_IN", guest.path("reply"))
        .env("LMX_TTY_OUT", guest.path("request"))
        .output()
        .expect("run pbpaste");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("or paste with Cmd+V."));
}

#[test]
fn pbpaste_needs_a_terminal() {
    let guest = Guest::new();
    let output = guest
        .alias("pbpaste", &[])
        .env("LMX_TTY_IN", guest.path("missing/tty"))
        .output()
        .expect("run pbpaste");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbpaste needs the terminal of a limanix shell session.\n"
    );
}

#[test]
fn clipboard_subcommands_copy_and_paste() {
    let guest = Guest::new();
    guest.write("tty", "");
    let output = run_with_input(
        guest
            .command(&["clipboard", "copy"], &guest.config())
            .env("LMX_TTY_OUT", guest.path("tty")),
        b"hello\n",
    );
    assert!(output.status.success());
    assert_eq!(
        fs::read(guest.path("tty")).expect("read the terminal"),
        b"\x1b]52;c;aGVsbG8K\x07"
    );

    guest.write("reply", "\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
    guest.write("request", "");
    let output = guest
        .command(&["clipboard", "paste"], &guest.config())
        .env("LMX_TTY_IN", guest.path("reply"))
        .env("LMX_TTY_OUT", guest.path("request"))
        .output()
        .expect("run lmx clipboard paste");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello world");
}

#[test]
fn clipboard_goes_through_tmux() {
    let guest = Guest::new();
    let bin = fake_tmux(&guest, true);
    let output = run_with_input(in_tmux(&mut guest.alias("pbcopy", &[]), &bin), b"copied");
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(guest.path("tmux/buffer")).expect("read the buffer"),
        "copied"
    );

    let output = in_tmux(&mut guest.alias("pbpaste", &[]), &bin)
        .output()
        .expect("run pbpaste");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pasted");
    let calls = fs::read_to_string(guest.path("tmux/calls")).expect("read the calls");
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "load-buffer -w -",
            "list-buffers -F #{buffer_created} #{buffer_name}",
            "refresh-client -l",
            "list-buffers -F #{buffer_created} #{buffer_name}",
            "save-buffer -b buffer1 -",
        ]
    );
}

#[test]
fn pbpaste_through_tmux_gives_up_after_the_timeout() {
    let guest = Guest::new();
    let bin = fake_tmux(&guest, false);
    let output = in_tmux(&mut guest.alias("pbpaste", &[]), &bin)
        .env("LMX_PASTE_TIMEOUT", "1")
        .output()
        .expect("run pbpaste");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("or paste with Cmd+V."));
}

#[test]
fn a_closed_standard_output_ends_quietly() {
    let guest = Guest::new();
    for args in [
        &["status"][..],
        &["status", "--json"],
        &["version"],
        &["info"],
        &["welcome"],
        &[],
    ] {
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

#[test]
fn doctor_without_lmxd_reports_the_owner_and_reads_the_markers() {
    let guest = Guest::new();
    let output = guest.lmx(&["doctor", "--json"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
    let checks: Vec<(&str, &str)> = answer["data"]["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .map(|check| {
            (
                check["check"].as_str().unwrap_or_default(),
                check["status"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        checks,
        [
            ("config", "ok"),
            ("owner", "failed"),
            ("generations", "warning")
        ]
    );

    let short = guest.lmx(&["status", "--short"], &guest.config());
    assert!(short.status.success(), "{short:?}");
    assert_eq!(String::from_utf8_lossy(&short.stdout), "lmxd?\n");
}

#[test]
fn net_check_finds_a_listener_that_only_the_guest_reaches() {
    let guest = Guest::new();
    guest.write("proc/net/tcp", TCP);
    guest.write("proc/4242/comm", "python3\n");
    fs::create_dir_all(guest.path("proc/4242/fd")).expect("create fd");
    symlink("socket:[4242]", guest.path("proc/4242/fd/3")).expect("link the socket");
    guest.write(
        "etc/passwd",
        "root:x:0:0::/root:/bin/sh\ndev:x:1000:100::/home/dev:/bin/sh\n",
    );

    let output = guest.lmx(&["net", "check", "8080", "--json"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
    assert_eq!(
        answer["data"],
        json!({
            "port": 8080,
            "protocol": "tcp",
            "checks": [
                {
                    "check": "firewall",
                    "status": "ok",
                    "message": "TCP 8080 is open in the guest firewall."
                },
                {
                    "check": "listener",
                    "status": "failed",
                    "message": "TCP 8080 listens on 127.0.0.1 only and is reachable only inside the guest.",
                    "hint": "Make the application listen on 0.0.0.0 or the guest address."
                },
                {
                    "check": "process",
                    "status": "ok",
                    "message": "python3 (pid 4242) holds the socket of user dev."
                }
            ]
        })
    );
}

#[test]
fn logs_show_the_latest_run_of_a_kind() {
    let guest = Guest::new();
    let output = guest.lmx(&["logs", "apply"], &guest.config());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "system-apply-2, generation 0123456789ab, 2026-10-07 09:12:03 UTC\n\
         applying generation 0123456789ab\n\
         building the system configuration...\n"
    );

    let output = guest.lmx(&["logs", "health"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run sudo lmx logs health"),
        "{output:?}"
    );
}
