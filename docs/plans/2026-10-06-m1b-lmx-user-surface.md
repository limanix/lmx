# M1b: lmx User Surface Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move the guest's user commands from the platform's shell scripts into the `lmx` binary: the help page,
`lmx info`, the shell welcome, the Mac clipboard (`pbcopy`, `pbpaste`) and named sessions (`limanix-session`). The
replaced commands keep their names, arguments, messages and exit statuses.

**Architecture:**
- `lmx-model` gains `Version`, the `lmx version` answer, so every contract value lives there.
- `lmx-facts` gains two readers: `mounts` (shared folders from `/proc/self/mountinfo`) and `machine` (processors,
  memory from `/proc/meminfo`, the kernel from `uname`).
- The `lmx` binary gains:
  - guest pages for people, `help`, `info` and `welcome`, rendered with two small text modules: `layout` (terminal
    columns, wrapping, labeled rows) and `palette` (Catppuccin Mocha colors, only on a terminal);
  - caller commands: `clipboard` (OSC 52 through `/dev/tty`, or tmux inside tmux) and `session` (replaces the process
    with the configured provider);
  - multi-call dispatch: the file name the binary is started with selects `pbcopy`, `pbpaste` or `limanix-session`,
    whose arguments are parsed by hand to keep their old syntax.
- The host contract is unchanged; `contract/v1/` gains the `lmx version` example.

**Tech Stack:**
- Rust 1.90.0, edition 2024; clap 4.6; rustix 1.1 with `event`, `system` and `termios`; base64 0.23; unicode-width 0.2
- Task 3.53.1 with `mr-chelyshkin/tasks` v0.0.5 and the `ci/rust` image, as in M1a

---

## Before you start

- **Design and history.** [`2026-10-06-guest-owner-design.md`](2026-10-06-guest-owner-design.md) describes the guest
  owner. [`2026-10-06-m1a-lmx-foundation.md`](2026-10-06-m1a-lmx-foundation.md) built the workspace this plan extends;
  its "Out of scope" and "Follow-ups from the final review" sections list what M1b owes. M1c, the client and the
  platform, has its own plan.
- **Repository and branch.** `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx`, branch `feat/m1a-foundation`:
  M1b continues on the M1a branch. Paths under **Files** are relative to the repository root; commands use absolute
  paths.
- **Behavior to keep.** The replaced scripts live in
  `/Users/igoss/Desktop/lima-personal-shared/limanix/client/internal/nixos/resources/base/`: `lmx.sh`, `help.sh`,
  `info.sh`, `welcome.sh`, `pbcopy.sh`, `pbpaste.sh` and `session.sh`. Their Go tests in
  `client/internal/nixos/{workspace,clipboard,session}_test.go` pin the semantics:
  - `limanix-session` passes its one argument to the provider unchanged, even `--help`, and the provider owns the
    streams and the exit status. Without a provider it exits 127 and suggests catalog providers only when there are
    some.
  - `pbcopy` writes `ESC ] 52 ; c ; <base64> BEL` to the terminal; inside tmux it runs `tmux load-buffer -w -`.
  - `pbpaste` asks with `ESC ] 52 ; c ; ? BEL` without echo or line buffering and restores the terminal afterwards.
    Inside tmux it asks tmux with `refresh-client -l` and prints the buffer that appears.
  - Extra arguments are usage errors with exit status 2.
- **Deliberate differences from the scripts:**
  - `lmx help` prints the command reference even when the workspace metadata is unreadable, and still exits 1; the
    explanation is part of the page on standard output, where `help.sh` wrote a short error to standard error;
  - usage errors of `lmx` itself are clap's messages with the usage of the command; the exit status stays 2;
  - `lmx info` shows labeled rows instead of raw `df`, `findmnt` and `systemctl` output, and shows every part before
    it exits 1 for an unreadable one;
  - `pbpaste` also accepts replies ended by `ESC \`, discards keys typed around the reply in the same read, reports
    `?` as unanswered, and inside tmux prints the new buffer by name rather than whichever buffer is newest;
  - `limanix-session` reads its provider from `/etc/lmx/config.json` instead of values baked into the script; when
    the file is unreadable it exits 1 with `lmx: cannot read …`, while the welcome shows `unknown` and exits 0;
  - help for one command, such as `lmx info --help`, is clap's and exits 0; the scripts printed their usage line
    and exited 2;
  - column widths follow Unicode East Asian Width instead of counting UTF-8 bytes;
  - sizes of 10 GiB and more are rounded once from bytes, as in `lmx status`: 10.47 GiB shows as `10 GiB`, where the
    script rounded tenths first and showed `11 GiB`;
  - a word that ends exactly at the wrap width stays on its line, so some long welcome lines break one word later;
  - `lmx welcome` exits 1 when the mount table is unreadable; the script passed on the status of `findmnt` or `jq`,
    which the shell init ignores either way;
  - `welcome` waits at most 3 seconds for `systemctl`, the tool timeout of M1a; the script had no limit.
- **Shell quirk.** As in M1a, `cd` into the limanix directories triggers a zsh GVM hook. Use absolute paths, `git -C`,
  `task --dir` and `cargo +1.90.0 <subcommand> --manifest-path`.
- **Checks.** `cargo` 1.90.0 on the host is the fast loop. `task --dir <repo> --yes ci/<check>` runs the CI
  equivalent in `ghcr.io/mr-chelyshkin/ci/rust:1.90.0` and needs Docker.
- **Style and lints** are those of M1a:
  - `//!` documentation on every crate root and module, `///` on every item including private fields;
  - `#![forbid(unsafe_code)]` in every crate root;
  - test names are sentences about behavior;
  - manifests align `=` and pin full versions;
  - CI denies `missing_docs`, `clippy::missing_docs_in_private_items`, `unreachable_pub` and
    `missing_debug_implementations`.
- **Documentation lands once.** The crate documentation of `crates/lmx/src/main.rs`, `README.md` and
  `ARCHITECTURE.md` change in Task 11, after every command exists.
- **`Cargo.lock`.** Tasks 6 and 10 add dependencies; keep the lock file changes Cargo makes.
- **Commits are made by the user.** Each task ends with a suggested commit. Implementers stop there and leave the
  changes uncommitted: never stage, commit, push or open a pull request.

---

## Task 0: Starting point

**Step 1: Check the branch and the working tree**

Run: `git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx status --short --branch`

Expected: `## feat/m1a-foundation`, and at most the untracked plan `docs/plans/2026-10-06-m1b-lmx-user-surface.md`.

**Step 2: Run the M1a suite**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace`

Expected: PASS, 38 tests.

---

## Task 1: `Version` in `lmx-model`

The `lmx version` answer is the last contract value that lives in the binary. Moving it next to `Status` lets the
client's contract tests decode it from the same crate and example directory.

**Files:**
- Create: `contract/v1/version.json`, `crates/lmx-model/src/version.rs`
- Modify: `crates/lmx-model/src/lib.rs`, `crates/lmx/src/version.rs`, `docs/contract.md`

**Step 1: Publish the example**

Create `contract/v1/version.json`, indented like the status examples:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "version": "0.1.0",
    "contract": 1
  }
}
```

**Step 2: Write the failing round-trip test**

Create `crates/lmx-model/src/version.rs`:

```rust
//! Answer of `lmx version`: a release and the host contract it speaks.

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Envelope, Version};

    /// The published example of contract version 1 is a successful answer that decodes and encodes
    /// without loss.
    #[test]
    fn contract_example_round_trips() {
        let example = include_str!("../../../contract/v1/version.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Version> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert!(envelope.ok && envelope.error.is_none());
        assert_eq!(
            envelope.data.as_ref().map(|version| version.contract),
            Some(CONTRACT_VERSION)
        );
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
```

Register the module in `crates/lmx-model/src/lib.rs`: add a row to the table in the crate documentation, the module and
the re-export.

After the `Status` row:

```rust
//! | [`Version`]  | `lmx version`                           | the host and people     |
```

After `mod status;`:

```rust
mod version;
```

After the `status` re-export:

```rust
pub use version::Version;
```

**Step 3: Run the test to see it fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: FAIL to compile: `unresolved import` of `version::Version`.

**Step 4: Add the type**

Add the implementation between the module documentation and the tests:

```rust
use serde::{Deserialize, Serialize};

/// Release of a binary and the host contract it speaks.
///
/// The host reads it before other answers, so it can refuse a guest whose contract it does not know
/// instead of misreading its answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Release version of the binary, such as `0.1.0`.
    pub version: String,
    /// Host contract version the binary speaks.
    pub contract: u32,
}
```

**Step 5: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: PASS, 11 tests.

**Step 6: Answer with the shared type**

Replace `crates/lmx/src/version.rs` with:

```rust
//! `lmx version`: the binary and the host contract it speaks.

use std::{io, process::ExitCode};

use lmx_model::{CONTRACT_VERSION, Envelope, Version};

use crate::{cli::OutputArgs, output};

/// Runs `lmx version`.
pub(crate) fn run(args: &OutputArgs) -> io::Result<ExitCode> {
    let version = Version {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract: CONTRACT_VERSION,
    };
    if args.json {
        output::write_json(&Envelope::success(version))?;
    } else {
        output::write_text(&format!(
            "lmx {} (host contract {})\n",
            version.version, version.contract
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}
```

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS; `version_json_names_the_contract` still sees the same answer.

**Step 7: Link the example from the contract**

In `docs/contract.md`, after the sentence about `data` of `lmx version`:

```markdown
Example: [version](../contract/v1/version.json).
```

**Step 8: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add contract/v1/version.json crates/lmx-model/src crates/lmx/src/version.rs docs/contract.md
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Move the version answer into lmx-model"
```

---

## Task 2: Shared folders (`lmx-facts::mounts`)

`lmx info` and the welcome list the Mac folders shared into the guest. The kernel's mount table replaces `findmnt` and
`jq`, so the reader needs no tools and parses in-process. As in `findmnt`, a folder is read-only when its mount or its
file system is, and a mount point that is not UTF-8 does not hide the others.

**Files:**
- Create: `crates/lmx-facts/src/mounts.rs`
- Modify: `crates/lmx-facts/src/lib.rs`, `crates/lmx-facts/src/error.rs`

**Step 1: Write the failing tests**

Create `crates/lmx-facts/src/mounts.rs`:

```rust
//! Shared folders: the virtiofs and 9p mounts of the guest.
//!
//! LimaNix shares Mac folders with virtiofs under Apple's virtualization and with 9p under QEMU.
//! The kernel's mount table lists every mount of the calling process with its type and options.

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn keeps_shared_folders_with_their_mode() {
        let table = "\
22 1 252:2 / / rw,noatime shared:1 - ext4 /dev/vda2 rw
41 22 0:38 / /home/dev rw,relatime shared:20 - virtiofs mount0 rw
42 22 0:39 / /mnt/limanix ro,relatime shared:21 - virtiofs mount1 rw
43 22 0:40 / /work\\040space rw,relatime - 9p mount2 rw,trans=virtio
44 22 0:41 / /srv/shared rw,relatime - virtiofs mount3 ro
";
        assert_eq!(
            parse(table).expect("valid mount table"),
            [
                Mount {
                    target: "/home/dev".into(),
                    fs_type: "virtiofs".into(),
                    read_only: false
                },
                Mount {
                    target: "/mnt/limanix".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true
                },
                Mount {
                    target: "/work space".into(),
                    fs_type: "9p".into(),
                    read_only: false
                },
                Mount {
                    target: "/srv/shared".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true
                },
            ]
        );
    }

    #[test]
    fn keeps_text_that_is_not_an_escape() {
        assert_eq!(unescape(r"/a\b\0x9\400"), r"/a\b\0x9\400");
        assert_eq!(unescape(r"/tab\011and\134slash"), "/tab\tand\\slash");
        assert_eq!(unescape(r"/a\+12b"), r"/a\+12b");
    }

    #[test]
    fn keeps_shared_folders_beside_a_mount_point_that_is_not_utf8() {
        let mut table = tempfile::NamedTempFile::new().expect("temporary mount table");
        table
            .write_all(
                b"\
30 22 8:1 / /media/caf\xe9 rw - ext4 /dev/sdb1 rw
41 22 0:38 / /home/dev rw - virtiofs mount0 rw
",
            )
            .expect("write the mount table");
        let mounts = shared(table.path()).expect("readable mount table");
        assert_eq!(
            mounts
                .iter()
                .map(|mount| mount.target.as_str())
                .collect::<Vec<_>>(),
            ["/home/dev"]
        );
    }

    #[test]
    fn rejects_lines_without_a_separator() {
        let error = parse("22 1 252:2 / / rw ext4 /dev/vda2 rw").expect_err("no separator");
        assert!(
            error
                .to_string()
                .starts_with("unexpected mount table output"),
            "{error}"
        );
    }

    #[test]
    fn explains_an_unreadable_table() {
        let error = shared(Path::new("/nonexistent/mountinfo")).expect_err("missing table");
        assert!(
            error
                .to_string()
                .starts_with("cannot read the mount table: "),
            "{error}"
        );
    }
}
```

Register the module in `crates/lmx-facts/src/lib.rs`.

After the `generations` row of the table:

```rust
//! | [`mounts`]      | shared folders and their mode           | `/proc/self/mountinfo`                  |
```

After `pub mod generations;`:

```rust
pub mod mounts;
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts mounts`

Expected: FAIL to compile: `parse`, `shared`, `unescape` and `Mount` do not exist.

**Step 3: Read the mount table**

Add the implementation between the module documentation and the tests:

```rust
use std::{fs, path::Path};

use crate::FactError;

/// Mount table of the calling process inside a booted guest.
pub const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// File-system types that carry shared folders.
const SHARED_TYPES: [&str; 2] = ["virtiofs", "9p"];

/// One shared folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Mount point in the guest.
    pub target: String,
    /// File-system type: `virtiofs` or `9p`.
    pub fs_type: String,
    /// Whether the guest can only read the folder.
    pub read_only: bool,
}

/// Reads the shared folders from the mount table at `path`.
///
/// A mount point that is not UTF-8 does not hide the others: its invalid bytes become U+FFFD.
pub fn shared(path: &Path) -> Result<Vec<Mount>, FactError> {
    let table = fs::read(path).map_err(|source| FactError::Io {
        what: "the mount table",
        source,
    })?;
    parse(&String::from_utf8_lossy(&table))
}

/// Parses `/proc/self/mountinfo`, keeping virtiofs and 9p mounts.
///
/// Each line is `id parent major:minor root target options [optional fields] - type source
/// super-options`. The kernel writes spaces, tabs, newlines and backslashes in the target as octal
/// escapes such as `\040`.
pub fn parse(table: &str) -> Result<Vec<Mount>, FactError> {
    let mut mounts = Vec::new();
    for line in table.lines().filter(|line| !line.trim().is_empty()) {
        let malformed = || FactError::Parse {
            what: "mount table",
            detail: line.to_owned(),
        };
        let (head, tail) = line.split_once(" - ").ok_or_else(malformed)?;
        let fields: Vec<&str> = head.split(' ').collect();
        let (Some(target), Some(options)) = (fields.get(4), fields.get(5)) else {
            return Err(malformed());
        };
        let mut tail = tail.split(' ');
        let fs_type = tail.next().unwrap_or_default();
        let super_options = tail.nth(1).unwrap_or_default();
        if SHARED_TYPES.contains(&fs_type) {
            mounts.push(Mount {
                target: unescape(target),
                fs_type: fs_type.to_owned(),
                // As in findmnt: read-only when the mount or its file system is.
                read_only: has_ro(options) || has_ro(super_options),
            });
        }
    }
    Ok(mounts)
}

/// Whether comma-separated mount options include `ro`.
fn has_ro(options: &str) -> bool {
    options.split(',').any(|option| option == "ro")
}

/// Decodes the kernel's three-digit octal escapes; anything else is kept as written.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'\\')
            .then(|| bytes.get(index + 1..index + 4))
            .flatten()
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 4;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}
```

`FactError::Parse` now also reports a malformed kernel file. In `crates/lmx-facts/src/error.rs`, replace its doc
comment:

```rust
    /// A program's output did not have the expected shape.
```

with:

```rust
    /// A program's output or a system file did not have the expected shape.
```

**Step 4: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts mounts`

Expected: PASS, 5 tests.

**Step 5: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-facts/src
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Read shared folders from the mount table"
```

---

## Task 3: Processors, memory and kernel (`lmx-facts::machine`)

The welcome shows processors, memory and free disk; `lmx info` shows the kernel. This task also fixes two `lmx-facts`
docs from the M1a review and a test race seen on macOS.

**Files:**
- Create: `crates/lmx-facts/src/machine.rs`
- Modify: `Cargo.toml`, `crates/lmx-facts/src/lib.rs`, `crates/lmx-facts/src/generations.rs`, `crates/lmx-facts/src/command.rs`

**Step 1: Enable `uname` in rustix**

In the root `Cargo.toml`, replace:

```toml
rustix     = { version = "1.1.4", features = ["fs"] }
```

with:

```toml
rustix     = { version = "1.1.4", features = ["fs", "system"] }
```

**Step 2: Write the failing tests**

Create `crates/lmx-facts/src/machine.rs`:

```rust
//! The virtual machine itself: processors, memory and kernel.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_total_memory_in_bytes() {
        let info = "MemTotal:        7969124 kB\nMemFree:          524288 kB\n";
        assert_eq!(parse_memory(info).expect("MemTotal"), 7_969_124 * 1024);
    }

    #[test]
    fn rejects_memory_information_without_a_total() {
        let error = parse_memory("MemFree: 524288 kB\n").expect_err("no MemTotal");
        assert!(
            error
                .to_string()
                .starts_with("unexpected memory information output"),
            "{error}"
        );
    }

    #[test]
    fn explains_unreadable_memory_information() {
        let error = memory(Path::new("/nonexistent/meminfo")).expect_err("missing file");
        assert!(
            error
                .to_string()
                .starts_with("cannot read memory information: "),
            "{error}"
        );
    }

    #[test]
    fn counts_at_least_one_processor() {
        assert!(cpus().expect("processor count") >= 1);
    }

    #[test]
    fn names_the_running_kernel() {
        let kernel = kernel();
        assert!(kernel.contains(' ') && !kernel.starts_with(' '), "{kernel}");
    }
}
```

Register the module in `crates/lmx-facts/src/lib.rs`.

After the `generations` row of the table:

```rust
//! | [`machine`]     | processors, memory and kernel           | the scheduler, `/proc/meminfo`, `uname` |
```

After `pub mod generations;`:

```rust
pub mod machine;
```

**Step 3: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts machine`

Expected: FAIL to compile: `parse_memory`, `memory`, `cpus`, `kernel` and the `Path` import do not exist yet.

**Step 4: Read the machine**

Add the implementation between the module documentation and the tests:

```rust
use std::{fs, path::Path, thread};

use crate::FactError;

/// Memory information inside a booted guest.
pub const MEMINFO_PATH: &str = "/proc/meminfo";

/// Number of processors the caller may use.
pub fn cpus() -> Result<usize, FactError> {
    thread::available_parallelism()
        .map(usize::from)
        .map_err(|source| FactError::Io {
            what: "the processor count",
            source,
        })
}

/// Reads total memory, in bytes, from the memory information at `path`.
pub fn memory(path: &Path) -> Result<u64, FactError> {
    let info = fs::read_to_string(path).map_err(|source| FactError::Io {
        what: "memory information",
        source,
    })?;
    parse_memory(&info)
}

/// Parses `MemTotal` from `/proc/meminfo`, which the kernel reports in kibibytes.
pub fn parse_memory(info: &str) -> Result<u64, FactError> {
    info.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| value.trim().strip_suffix("kB"))
        .and_then(|kibibytes| kibibytes.trim().parse::<u64>().ok())
        .map(|kibibytes| kibibytes.saturating_mul(1024))
        .ok_or_else(|| FactError::Parse {
            what: "memory information",
            detail: "no MemTotal line in kB".into(),
        })
}

/// Kernel name and release, such as `Linux 6.12.5`.
pub fn kernel() -> String {
    let name = rustix::system::uname();
    format!(
        "{} {}",
        name.sysname().to_string_lossy(),
        name.release().to_string_lossy()
    )
}
```

**Step 5: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts machine`

Expected: PASS, 5 tests.

**Step 6: Fix two docs from the M1a review**

The runners also take `PATH` names when the configuration is unreadable. In `crates/lmx-facts/src/lib.rs`, replace:

```rust
//! Readers that run a program take its absolute path from the platform configuration and split
//! process I/O from a pure parser, so the parsers are tested with fixed output.
```

with:

```rust
//! Readers that run a program take its path from the caller: an absolute path from the platform
//! configuration, or a `PATH` name when the configuration is unreadable. They split process I/O
//! from a pure parser, so the parsers are tested with fixed output.
```

`GenerationPaths::under` cannot inspect another machine's tree. In `crates/lmx-facts/src/generations.rs`, replace:

```rust
    /// Marker locations below `root`, for tests and offline inspection.
```

with:

```rust
    /// Marker locations below `root`, for tests.
    ///
    /// Not for inspecting another machine's tree: the system profile and `/run/booted-system` are
    /// absolute symlinks into `/nix/store`, which resolve on the inspecting machine.
```

**Step 7: Keep the hanging tool of the runner test short**

On macOS, Rust creates a pipe first and marks it close-on-exec afterwards, because macOS has no `pipe2`. A process that
another test thread spawns in between inherits the pipe, and the pipe's reader then waits until that process exits.
`stops_waiting_for_a_tool_that_hangs` spawns `sleep 5`, longer than the 3-second `TIMEOUT` of the other runner tests, so
on a Mac one of them occasionally fails with `did not finish within 3s`. Linux, and with it CI, creates pipes
atomically.

In `crates/lmx-facts/src/command.rs`, replace the test:

```rust
    #[test]
    fn stops_waiting_for_a_tool_that_hangs() {
        let started = Instant::now();
        let error = shell("exec sleep 5", Duration::from_millis(100)).expect_err("sleep hangs");
        assert!(matches!(error, FactError::Timeout { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
```

with:

```rust
    #[test]
    fn stops_waiting_for_a_tool_that_hangs() {
        // The sleep stays shorter than `TIMEOUT`. On macOS a pipe becomes close-on-exec only after
        // it is created, so a pipe another test creates at that moment can leak into the sleep,
        // and that test then waits for the sleep to exit.
        let started = Instant::now();
        let error = shell("exec sleep 2", Duration::from_millis(100)).expect_err("sleep hangs");
        assert!(matches!(error, FactError::Timeout { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_millis(1500));
    }
```

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts command`

Expected: PASS, 5 tests.

**Step 8: Build the documentation**

Run: `RUSTDOCFLAGS='-D warnings' cargo +1.90.0 doc --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace --no-deps --document-private-items`

Expected: no warnings; the new table rows link to `machine` and `mounts`.

**Step 9: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add Cargo.toml crates/lmx-facts/src
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Read processors, memory and the kernel"
```

---

## Task 4: Fake tools that macOS has already checked

macOS checks a new executable the first time it runs. On a development Mac the first run of a freshly written script
takes 0.2–0.4 s instead of 0.01 s, and with two dozen new scripts started at once the checks queue up to about 6 s.
Every integration test writes its own fake `ip` and `systemctl`, and `lmx` waits at most 3 s for a tool, so a test
occasionally loses a fact: `welcome_summarizes_the_vm` of Task 8 then misses its failed unit. Linux, and with it CI, has
no such check. M1b doubles the integration tests, so the fake tools become shared: written once per test run under
`CARGO_TARGET_TMPDIR` and run once before any test uses them.

**Files:**
- Modify: `crates/lmx/tests/cli.rs`

**Step 1: Share the fake tools**

In `crates/lmx/tests/cli.rs`, add `process` itself and `sync::OnceLock` to the `std` import. Replace:

```rust
    process::{Command, Output},
};
```

with:

```rust
    process::{self, Command, Output},
    sync::OnceLock,
};
```

Before `Guest`, add:

```rust
/// Paths of the fake `ip` and `systemctl`.
struct Tools {
    /// Prints one interface with a global IPv4 address.
    ip: PathBuf,
    /// Prints one failed unit.
    systemctl: PathBuf,
}

/// Fake tools shared by every test.
///
/// macOS checks a new executable the first time it runs. With many new scripts at once the check can
/// outlast the 3-second tool timeout of `lmx`, so the tools are written once and run once before a
/// test needs them.
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
        }
    })
}

/// Installs an executable that prints `output` into `directory`, runs it once and returns its path.
///
/// The script uses only shell built-ins because the tests run `lmx` with an empty `PATH`. Another
/// test run, such as one an IDE starts, may be running the tool at the same time, so an outdated
/// tool is replaced whole and a current one is left alone.
fn fake_tool(directory: &Path, name: &str, output: &str) -> PathBuf {
    assert!(!output.contains('\''), "tool output is single-quoted");
    let path = directory.join(name);
    let script = format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n");
    let current = fs::read_to_string(&path).ok().as_deref() == Some(script.as_str())
        && fs::metadata(&path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    if !current {
        let staged = directory.join(format!(".{name}.{}", process::id()));
        fs::create_dir_all(directory).expect("create the tool directory");
        fs::write(&staged, &script).expect("write the tool");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
            .expect("make the tool executable");
        fs::rename(&staged, &path).expect("install the tool");
    }
    let first_run = Command::new(&path).output().expect("run the tool");
    assert!(first_run.status.success(), "{name} runs");
    path
}
```

The tree no longer holds the tools. Replace the doc comment of `Guest`:

```rust
/// A guest tree with generation markers, a store directory, fake tools and a configuration.
```

with:

```rust
/// A guest tree with generation markers, a store directory and a configuration that names the
/// fake tools.
```

In `Guest::new`, replace the two `guest.tool` calls:

```rust
        let ip = guest.tool(
            "ip",
            r#"[{"ifname":"enp0s1","address":"52:55:55:aa:bb:cc","addr_info":[{"family":"inet","scope":"global","local":"192.0.2.10"}]}]"#,
        );
        let systemctl = guest.tool(
            "systemctl",
            "limanix-store-guard.service loaded failed failed Guard",
        );
```

with:

```rust
        let tools = tools();
```

and the `tools` entry of the configuration:

```rust
            "tools": {"ip": ip, "systemctl": systemctl}
```

with:

```rust
            "tools": {"ip": tools.ip, "systemctl": tools.systemctl}
```

Remove `Guest::tool`, which `fake_tool` replaces:

```rust
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
```

**Step 2: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli`

Expected: PASS, 7 tests.

**Step 3: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/tests/cli.rs
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Run the fake tools once before the tests use them"
```

---

## Task 5: The guest help page

The platform's shell `lmx` printed a help page for `lmx`, `lmx help`, `lmx --help` and `lmx -h`, and for
`lmx help --help` and `lmx help -h`; any other argument was a usage error. M1a prints clap's generated help and rejects
`lmx help`. This task brings the page back: the workspace summary, then the reference of `help.sh` with one new line for
`lmx status`. Help for one command, such as `lmx status --help`, stays generated.

**Files:**
- Create: `crates/lmx/src/help.rs`
- Modify: `crates/lmx/src/output.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`
- Test: `crates/lmx/tests/cli.rs`

**Step 1: Write the failing tests**

In `crates/lmx/tests/cli.rs`, give `Guest` a path helper after `write`:

```rust
    /// Path of `relative` below the root.
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }
```

Replace the test `no_command_prints_help`:

```rust
#[test]
fn no_command_prints_help() {
    let guest = Guest::new();
    let output = guest.lmx(&[], &guest.config());
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: lmx"));
}
```

with:

```rust
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
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli help`

Expected: FAIL: `every_way_of_asking_for_help_prints_the_guest_page` and `help_takes_no_other_arguments` get clap's
help, and `help_without_metadata_lists_commands_and_fails` gets exit status 2.

**Step 3: Give pages one exit status**

Pages for people print what they can and exit 1 when a part they need was unreadable: the metadata for `help`, any part
for `info`, and the mount table for the welcome.

In `crates/lmx/src/output.rs`, replace the `std` import:

```rust
use std::io::{self, Write};
```

with:

```rust
use std::{
    io::{self, Write},
    process::ExitCode,
};
```

Pages explain their failures themselves. Replace the doc comment of `FAILURE`:

```rust
/// Exit status of a failed operation; details are in the JSON answer or on standard error.
```

with:

```rust
/// Exit status of a failed operation; details are in the JSON answer, in the page, or on standard
/// error.
```

At the end of the file:

```rust
/// Exit status of a page for people: success when the parts the page needs could be read.
pub(crate) fn page_status(complete: bool) -> ExitCode {
    if complete {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(FAILURE)
    }
}
```

**Step 4: Write the page**

Create `crates/lmx/src/help.rs`. `metadata` and `modules` are shared with `lmx info` and the welcome in later tasks:

```rust
//! `lmx help`: the guest's help page.
//!
//! `lmx`, `lmx help`, `lmx --help` and `lmx -h` print this page, as the platform's shell `lmx` did.
//! Help for one command, such as `lmx status --help`, comes from the command line parser.

use std::{io, process::ExitCode};

use lmx_model::Config;

use crate::{output, system::System};

/// Commands people use inside the VM and on the Mac.
const REFERENCE: &str = "\
Inside this VM
  lmx info              Show the kernel, guest disk, shared folders and failed units.
  lmx status            Show generations, disk, network, failed units; sudo shows every fact.
  lmx welcome           Show the workspace welcome again.
  limanix-session NAME  Open a named session with the selected session provider.
  pbcopy < FILE         Copy to the Mac clipboard through the terminal.
  pbpaste               Print the Mac clipboard, if the terminal allows reads.
  exit                  Return to the Mac.

On the Mac
  limanix list                          Show VM state and network addresses.
  limanix shell NAME                    Open the guest user's login shell.
  limanix shell NAME --session PROJECT  Open a named project session.
  limanix update --config FILE          Apply resources, modules and environment.
  limanix stop NAME                     Stop the VM; keep its disk and home.

Tools come from the modules selected in your TOML configuration.
No session provider is required for a normal shell.
Guide: https://limanix.dev/categories/client/getting-started.html
";

/// Runs `lmx help`; fails after printing the page when the workspace metadata is unreadable.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let (page, readable) = page(&system.config);
    output::write_text(&page)?;
    Ok(output::page_status(readable))
}

/// The help page, and whether the workspace metadata in it was readable.
fn page(config: &Result<Config, String>) -> (String, bool) {
    let mut page = String::from("LimaNix workspace\n\n");
    page.push_str(&metadata(config));
    page.push('\n');
    page.push_str(REFERENCE);
    (page, config.is_ok())
}

/// The workspace summary lines, or why they cannot be shown.
pub(crate) fn metadata(config: &Result<Config, String>) -> String {
    match config {
        Ok(config) => summary(config),
        Err(message) => format!("Workspace metadata cannot be read: {message}\n"),
    }
}

/// The VM, its user and the selected modules, one per line.
fn summary(config: &Config) -> String {
    format!(
        "VM: {} ({})\nUser: {}\nHome: {}\nModules: {}\n",
        config.vm.name,
        config.vm.arch,
        config.user.name,
        config.user.home,
        modules(config)
    )
}

/// The selected catalog modules, or `none`.
pub(crate) fn modules(config: &Config) -> String {
    if config.modules.is_empty() {
        "none".to_owned()
    } else {
        config.modules.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use lmx_model::{Config, DiskPolicy, Session, Tools, User, Vm};

    use super::*;

    /// Configuration of a VM named `dev-box` with no modules.
    fn config() -> Config {
        Config {
            schema: 1,
            vm: Vm {
                name: "dev-box".into(),
                arch: "arm64".into(),
                system: "NixOS 26.05".into(),
            },
            generation: "0123456789ab".into(),
            user: User {
                name: "dev".into(),
                home: "/home/dev".into(),
                uid: 501,
            },
            modules: vec![],
            disk: DiskPolicy {
                collect_percent: 20,
                minimum_percent: 10,
            },
            session: Session {
                command: None,
                providers: vec!["lmx:tmux".into()],
            },
            tools: Tools {
                ip: "ip".into(),
                systemctl: "systemctl".into(),
            },
        }
    }

    #[test]
    fn shows_the_workspace_and_every_command() {
        let (page, readable) = page(&Ok(config()));
        assert!(readable);
        assert!(page.starts_with(
            "LimaNix workspace\n\nVM: dev-box (arm64)\nUser: dev\nHome: /home/dev\nModules: none\n\n"
        ));
        for command in [
            "lmx info",
            "lmx status",
            "lmx welcome",
            "limanix-session NAME",
            "pbcopy",
            "pbpaste",
            "limanix shell NAME",
            "limanix update --config FILE",
        ] {
            assert!(page.contains(command), "missing {command}");
        }
    }

    #[test]
    fn explains_unreadable_metadata_and_still_lists_commands() {
        let (page, readable) = page(&Err("cannot read /etc/lmx/config.json".into()));
        assert!(!readable);
        assert!(
            page.contains("Workspace metadata cannot be read: cannot read /etc/lmx/config.json")
        );
        assert!(page.contains("lmx info"));
    }

    #[test]
    fn lists_selected_modules() {
        let mut config = config();
        config.modules = vec!["lmx:console".into(), "lmx:go".into()];
        assert_eq!(modules(&config), "lmx:console, lmx:go");
    }
}
```

**Step 5: Add `lmx help` to the command line**

Replace `crates/lmx/src/cli.rs` with the version below. `print_help` goes away. clap cannot disable `--help` for `lmx`
alone, because `disable_help_flag` propagates to every subcommand, so `main` turns a leading `--help` or `-h` into
`help` before clap parses. The `help` subcommand takes its own hidden `-h`/`--help` instead of the generated one, so
further arguments stay usage errors.

```rust
//! Command-line interface of `lmx`.
//!
//! `lmx`, `lmx --help` and `lmx -h` print the guest help page instead of the generated one.

use clap::{Arg, ArgAction, Args, Parser, Subcommand};

/// Guest owner command of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmx", version, disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Command to run; without one, `lmx` prints the guest help page.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Commands of `lmx`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show the workspace and the commands inside this VM and on the Mac.
    // `lmx help --help` and `lmx help -h` print the page too, as the platform's shell `lmx` did.
    #[command(
        disable_help_flag = true,
        arg = Arg::new("help").short('h').long("help").action(ArgAction::SetTrue).hide(true)
    )]
    Help,
    /// Show generations, disk usage, network interfaces and failed units.
    Status(OutputArgs),
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
}

/// Output selection shared by commands that answer the host.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Answer with the JSON host contract instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}
```

**Step 6: Dispatch help in `main`**

In `crates/lmx/src/main.rs`, keep the crate documentation and replace everything after `#![forbid(unsafe_code)]` with:

```rust
mod cli;
mod format;
mod help;
mod output;
mod status;
mod system;
mod version;

use std::{env, ffi::OsString, io, iter, process::ExitCode};

use clap::Parser;

use crate::{
    cli::{Cli, Command},
    system::System,
};

fn main() -> ExitCode {
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();

    let result = lmx(arguments);

    match result {
        Ok(code) => code,
        // Standard output was closed by its reader, as in `lmx status | true`: nobody is left to
        // read an answer or an error. clap ends `--help` the same way.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lmx: {error}");
            ExitCode::from(output::FAILURE)
        }
    }
}

/// Runs `lmx` with the arguments after the program name.
fn lmx(mut arguments: Vec<OsString>) -> io::Result<ExitCode> {
    // `lmx --help` and `lmx -h` are `lmx help`, so further arguments stay usage errors, as in the
    // platform's shell `lmx`; `lmx status --help` stays generated.
    if let Some(first) = arguments.first_mut()
        && matches!(first.to_str(), Some("--help" | "-h"))
    {
        *first = OsString::from("help");
    }
    let cli = Cli::parse_from(iter::once(OsString::from("lmx")).chain(arguments));

    match cli.command {
        None | Some(Command::Help) => help::run(&System::from_environment()),
        Some(Command::Status(args)) => status::run(&System::from_environment(), &args),
        Some(Command::Version(args)) => version::run(&args),
    }
}
```

**Step 7: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 17 tests.

**Step 8: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/src crates/lmx/tests/cli.rs
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Print the guest help page"
```

---

## Task 6: `lmx info`

`lmx info` shows the workspace summary and then one labeled row per part: kernel, guest disk, shared folders and failed
units. A part that cannot be read says so in its row; the command still exits 1, as `info.sh` did. The free-space text
moves into `format::disk`, which also leaves out inodes on a file system without an inode table (an M1a follow-up); Task
7 reuses it for `lmx status`.

**Files:**
- Create: `crates/lmx/src/info.rs`, `crates/lmx/src/layout.rs`
- Modify: `Cargo.toml`, `crates/lmx/Cargo.toml`, `crates/lmx/src/format.rs`, `crates/lmx/src/system.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`
- Test: `crates/lmx/tests/cli.rs`

Free space is what `df` shows as available: the space users may still write, without the root reserve. `info.sh` showed
it, and the welcome shows it and warns by it, so the page the warning points to agrees with it.

**Step 1: Add `unicode-width`**

Column widths count CJK characters and emoji as two columns. In the root `Cargo.toml`, replace the body of
`[workspace.dependencies]`, keeping `=` aligned:

```toml
lmx-facts  = { path = "crates/lmx-facts" }
lmx-model  = { path = "crates/lmx-model" }
clap       = { version = "4.6.6", features = ["derive"] }
rustix     = { version = "1.1.4", features = ["fs", "system"] }
serde      = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
tempfile   = "3.27.0"
thiserror  = "2.0.20"
```

with:

```toml
lmx-facts     = { path = "crates/lmx-facts" }
lmx-model     = { path = "crates/lmx-model" }
clap          = { version = "4.6.6", features = ["derive"] }
rustix        = { version = "1.1.4", features = ["fs", "system"] }
serde         = { version = "1.0.229", features = ["derive"] }
serde_json    = "1.0.151"
tempfile      = "3.27.0"
thiserror     = "2.0.20"
unicode-width = "0.2.2"
```

In `crates/lmx/Cargo.toml`, replace the body of `[dependencies]`:

```toml
clap       = { workspace = true }
lmx-facts  = { workspace = true }
lmx-model  = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
```

with:

```toml
clap          = { workspace = true }
lmx-facts     = { workspace = true }
lmx-model     = { workspace = true }
serde         = { workspace = true }
serde_json    = { workspace = true }
unicode-width = { workspace = true }
```

**Step 2: Write the failing tests**

In `crates/lmx/tests/cli.rs`, add a mount table before `Guest` and mention it in the doc comment of `Guest`. Replace the
doc comment:

```rust
/// A guest tree with generation markers, a store directory and a configuration that names the
/// fake tools.
```

with:

```rust
/// Mount table with the root disk and two shared folders.
const MOUNTINFO: &str = "\
22 1 254:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw
40 22 0:38 / /home/dev rw,relatime shared:20 - virtiofs mount0 rw
41 22 0:39 / /mnt/limanix ro,relatime shared:21 - virtiofs mount1 ro
";

/// A guest tree with generation markers, a store directory, kernel tables and a configuration that
/// names the fake tools.
```

In `Guest::new`, after the booted-system marker:

```rust
            "run/booted-system/etc/lmx/config.json",
            r#"{"generation": "ba9876543210"}"#,
        );
```

add:

```rust
        guest.write("proc/self/mountinfo", MOUNTINFO);
```

Before `a_closed_standard_output_ends_quietly`, add:

```rust
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
```

In `a_closed_standard_output_ends_quietly`, add `info` to the commands:

```rust
    for args in [&["status"][..], &["status", "--json"], &["version"], &[]] {
```

with:

```rust
    for args in [
        &["status"][..],
        &["status", "--json"],
        &["version"],
        &["info"],
        &[],
    ] {
```

**Step 3: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli info`

Expected: FAIL: `info` is not a command yet (exit status 2).

**Step 4: Measure and align text**

Create `crates/lmx/src/layout.rs`. `rows` keeps continuation lines in the value column, as the row closure of
`status.rs` does; Task 7 replaces that closure. Task 8 adds wrapping.

```rust
//! Text layout in terminal columns.
//!
//! Widths follow Unicode East Asian Width, so CJK and emoji take two columns and aligned columns
//! stay aligned.

use unicode_width::UnicodeWidthStr;

/// Terminal columns taken by `text`.
pub(crate) fn columns(text: &str) -> usize {
    text.width()
}

/// Writes labeled rows: the label column is `label_width` wide, and every further line of a value
/// stays in the value column.
pub(crate) fn rows(rows: &[(&str, String)], label_width: usize) -> String {
    let mut text = String::new();
    for (label, value) in rows {
        let mut lines = value.lines();
        let first = lines.next().unwrap_or_default();
        let padding = label_width.saturating_sub(columns(label));
        text.push_str(&format!("{label}{:padding$}{first}\n", ""));
        for line in lines {
            text.push_str(&format!("{:label_width$}{line}\n", ""));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_wide_characters_as_two_columns() {
        assert_eq!(columns("lmx"), 3);
        assert_eq!(columns("界"), 2);
        assert_eq!(columns("Привет ✓"), 8);
    }

    #[test]
    fn keeps_continuation_lines_in_the_value_column() {
        let text = rows(
            &[
                ("Kernel", "Linux 6.12.5".into()),
                ("Failed units", "a.service\nb.service".into()),
            ],
            14,
        );
        assert_eq!(
            text,
            "Kernel        Linux 6.12.5\nFailed units  a.service\n              b.service\n"
        );
    }

    #[test]
    fn pads_labels_by_terminal_columns() {
        assert_eq!(rows(&[("界", "value".into())], 4), "界  value\n");
    }
}
```

**Step 5: Format disk usage**

In `crates/lmx/src/format.rs`, after the module documentation:

```rust
use lmx_model::DiskUsage;
```

After `gibibytes`:

```rust
/// Formats the space available to users, as `df` reports it, and, on file systems with an inode
/// table, free inodes.
pub(crate) fn disk(usage: &DiskUsage) -> String {
    let space = format!(
        "{} of {} free",
        gibibytes(usage.available_bytes),
        gibibytes(usage.bytes)
    );
    if usage.inodes == 0 {
        return space;
    }
    format!(
        "{space}, {} of {} inodes free",
        usage.free_inodes, usage.inodes
    )
}
```

At the end of the tests:

```rust
    #[test]
    fn leaves_out_inodes_without_an_inode_table() {
        let mut usage = DiskUsage {
            bytes: 16 * GIB,
            free_bytes: 9 * GIB,
            available_bytes: 8 * GIB,
            inodes: 1_048_576,
            free_inodes: 495_616,
        };
        assert_eq!(
            disk(&usage),
            "8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free"
        );
        usage.inodes = 0;
        usage.free_inodes = 0;
        assert_eq!(disk(&usage), "8.0 GiB of 16 GiB free");
    }
```

**Step 6: Locate the mount table**

In `crates/lmx/src/system.rs`, replace the `lmx_facts` import:

```rust
use lmx_facts::{disk::STORE_PATH, generations::GenerationPaths};
```

with:

```rust
use lmx_facts::{disk::STORE_PATH, generations::GenerationPaths, mounts::MOUNTINFO_PATH};
```

After `store`:

```rust
    /// Mount table of this process.
    pub(crate) fn mountinfo(&self) -> PathBuf {
        self.root.join(MOUNTINFO_PATH.trim_start_matches('/'))
    }
```

**Step 7: Write the page**

Create `crates/lmx/src/info.rs`:

```rust
//! `lmx info`: the workspace, kernel, guest disk, shared folders and failed units in detail.

use std::{io, process::ExitCode};

use lmx_facts::{
    FactError, disk, machine,
    mounts::{self, Mount},
    units,
};

use crate::{
    format, help,
    layout::{columns, rows},
    output,
    system::System,
};

/// Width of the label column.
const LABEL: usize = 14;

/// Runs `lmx info`; fails after printing when any part cannot be read.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let (page, complete) = page(system);
    output::write_text(&page)?;
    Ok(output::page_status(complete))
}

/// The info page, and whether every part of it could be read.
fn page(system: &System) -> (String, bool) {
    let disk = disk::usage(&system.store()).map(|usage| format::disk(&usage));
    let shared = mounts::shared(&system.mountinfo()).map(|mounts| shared_table(&mounts));
    let failed = units::failed(&system.systemctl()).map(|units| {
        if units.is_empty() {
            "none".to_owned()
        } else {
            units.join("\n")
        }
    });
    let complete = system.config.is_ok() && disk.is_ok() && shared.is_ok() && failed.is_ok();

    let mut text = help::metadata(&system.config);
    text.push('\n');
    text.push_str(&rows(
        &[
            ("Kernel", machine::kernel()),
            ("Disk", or_unavailable(disk)),
            ("Shared", or_unavailable(shared)),
            ("Failed units", or_unavailable(failed)),
        ],
        LABEL,
    ));
    (text, complete)
}

/// A fact, or why it is unavailable.
fn or_unavailable(fact: Result<String, FactError>) -> String {
    fact.unwrap_or_else(|error| format!("unavailable: {error}"))
}

/// One line per shared folder: target, type and mode, in aligned columns.
fn shared_table(mounts: &[Mount]) -> String {
    if mounts.is_empty() {
        return "(none)".to_owned();
    }
    let width = |column: fn(&Mount) -> &str| {
        mounts
            .iter()
            .map(|mount| columns(column(mount)))
            .max()
            .unwrap_or_default()
    };
    let (target_width, type_width) = (width(|mount| &mount.target), width(|mount| &mount.fs_type));
    mounts
        .iter()
        .map(|mount| {
            let target_padding = target_width - columns(&mount.target);
            let type_padding = type_width - columns(&mount.fs_type);
            let mode = if mount.read_only { "ro" } else { "rw" };
            format!(
                "{}{:target_padding$}  {}{:type_padding$}  {mode}",
                mount.target, "", mount.fs_type, ""
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_shared_folders() {
        let mounts = [
            Mount {
                target: "/home/dev".into(),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
            Mount {
                target: "/mnt/limanix".into(),
                fs_type: "9p".into(),
                read_only: true,
            },
        ];
        assert_eq!(
            shared_table(&mounts),
            "/home/dev     virtiofs  rw\n/mnt/limanix  9p        ro"
        );
        assert_eq!(shared_table(&mounts[1..]), "/mnt/limanix  9p  ro");
        assert_eq!(shared_table(&[]), "(none)");
    }
}
```

**Step 8: Add `lmx info` to the command line**

In `crates/lmx/src/cli.rs`, after the `Help` variant:

```rust
    /// Show the kernel, guest disk, shared folders and failed units.
    Info,
```

In `crates/lmx/src/main.rs`, after `mod help;`:

```rust
mod info;
mod layout;
```

In `lmx`, after the help arm:

```rust
        Some(Command::Info) => info::run(&System::from_environment()),
```

**Step 9: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 25 tests.

**Step 10: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add Cargo.toml Cargo.lock crates/lmx
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add lmx info"
```

---

## Task 7: `lmx status` follow-ups

Two M1a follow-ups: render inodes only when the file system has an inode table, and pin the name of the `disk` problem.
`status.rs` switches to `format::disk` and `layout::rows`, so its text also shows the available space, as `lmx info` and
the welcome do. The JSON answer keeps every field.

**Files:**
- Modify: `crates/lmx/src/status.rs`
- Test: `crates/lmx/src/status.rs`, `crates/lmx/tests/cli.rs`

**Step 1: Write the failing tests**

In the tests of `crates/lmx/src/status.rs`, `renders_every_fact_on_its_own_line` now expects the available space.
Replace its disk line:

```rust
             Disk          9.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
```

with:

```rust
             Disk          8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
```

Before `keeps_continuation_lines_of_a_problem_in_the_value_column`, add:

```rust
    #[test]
    fn leaves_out_inodes_of_a_file_system_without_an_inode_table() {
        let status = Status {
            disk: Some(DiskUsage {
                bytes: 16 << 30,
                free_bytes: 9 << 30,
                available_bytes: 8 << 30,
                inodes: 0,
                free_inodes: 0,
            }),
            ..Status::default()
        };
        assert!(
            render(&status).contains("\nDisk          8.0 GiB of 16 GiB free\n"),
            "{}",
            render(&status)
        );
    }
```

In `crates/lmx/tests/cli.rs`, after `status_text_is_for_people`. It passes already and guards the problem name the host
matches on:

```rust
#[test]
fn status_names_an_unreadable_disk() {
    let guest = Guest::new();
    fs::remove_dir_all(guest.path("nix/store")).expect("remove the store");
    let answer = answer(&guest.lmx(&["status", "--json"], &guest.config()));
    let problems = answer["data"]["problems"]
        .as_array()
        .expect("problems are listed");
    assert_eq!(answer["data"]["disk"], Value::Null);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0]["fact"], "disk");
}
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx status`

Expected: FAIL: `renders_every_fact_on_its_own_line` finds `9.0 GiB` free, and
`leaves_out_inodes_of_a_file_system_without_an_inode_table` also finds `, 0 of 0 inodes free`.

**Step 3: Render with the shared helpers**

In `crates/lmx/src/status.rs`, replace the `crate` import:

```rust
use crate::{cli::OutputArgs, format::gibibytes, output, system::System};
```

with:

```rust
use crate::{cli::OutputArgs, format, layout, output, system::System};
```

Replace `LABEL` and `render`:

```rust
/// Width of the label column in text output.
const LABEL: usize = 14;

/// Renders status as aligned text for people.
pub(crate) fn render(status: &Status) -> String {
    let unknown = || "unknown".to_owned();
    let generations = &status.generations;
    let mut rows = vec![
        (
            "Generation",
            format!(
                "desired {}, built {}, booted {}",
                generations.desired.clone().unwrap_or_else(unknown),
                generations.built.clone().unwrap_or_else(unknown),
                generations.booted.clone().unwrap_or_else(unknown),
            ),
        ),
        (
            "Disk",
            status.disk.as_ref().map_or_else(unknown, format::disk),
        ),
        (
            "Network",
            status
                .interfaces
                .as_ref()
                .map_or_else(unknown, |interfaces| {
                    let addressed: Vec<String> = interfaces
                        .iter()
                        .filter(|interface| !interface.ipv4.is_empty())
                        .map(|interface| format!("{} {}", interface.name, interface.ipv4.join(" ")))
                        .collect();
                    if addressed.is_empty() {
                        "no global IPv4 address".into()
                    } else {
                        addressed.join(", ")
                    }
                }),
        ),
        (
            "Failed units",
            status.failed_units.as_ref().map_or_else(unknown, |units| {
                if units.is_empty() {
                    "none".into()
                } else {
                    units.join(", ")
                }
            }),
        ),
    ];
    for problem in &status.problems {
        rows.push(("Problem", format!("{}: {}", problem.fact, problem.message)));
    }
    layout::rows(&rows, LABEL)
}
```

**Step 4: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 27 tests.

**Step 5: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/src/status.rs crates/lmx/tests/cli.rs
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Share disk text and rows with lmx status"
```

---

## Task 8: `lmx welcome`

The welcome is the summary an interactive shell prints when it starts. This is a port of `welcome.sh`: logo, VM,
resources, modules, shared folders, warnings and the next commands. Labels take 11 columns after a two-column indent,
values wrap so no line passes 80 columns, and colors are the Catppuccin Mocha values of the platform prompt, written
only to a terminal without `NO_COLOR` and with a `TERM` other than `dumb`. The palette stays built in until the theme
moves into the declaration (design, M4).

Facts are collected first and rendered by a pure function, so the layout is tested without a guest. Every fact is best
effort, as in the script. The command exits 1 only when the mount table cannot be read; `welcome.sh` failed then too.

**Files:**
- Create: `crates/lmx/src/palette.rs`, `crates/lmx/src/welcome.rs`
- Modify: `crates/lmx/src/layout.rs`, `crates/lmx/src/system.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`
- Test: `crates/lmx/tests/cli.rs`

**Step 1: Write the failing tests**

In `Guest::new` of `crates/lmx/tests/cli.rs`, after the mount table:

```rust
        guest.write(
            "proc/meminfo",
            "MemTotal:        7969124 kB\nMemFree:         5123456 kB\n",
        );
```

Before `a_closed_standard_output_ends_quietly`, add:

```rust
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
```

In `a_closed_standard_output_ends_quietly`, after `info` in the commands:

```rust
        &["welcome"],
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli welcome`

Expected: FAIL: `welcome` is not a command yet (exit status 2).

**Step 3: Wrap text**

In `crates/lmx/src/layout.rs`, replace the end of the module documentation:

```rust
//! Widths follow Unicode East Asian Width, so CJK and emoji take two columns and aligned columns
//! stay aligned.
```

with:

```rust
//! Widths follow Unicode East Asian Width, so CJK and emoji take two columns and wrapped text never
//! runs past the edge of the screen.
```

After `columns`:

```rust
/// Splits `text` into lines of at most `width` columns.
///
/// A line breaks at its last space when it has one and never splits a character; spaces at a break
/// are dropped. A character wider than `width` gets a line of its own.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        // Prefixes are measured whole, as `columns` measures: joined emoji are narrower than the
        // sum of their characters.
        let end = rest
            .char_indices()
            .map(|(index, character)| index + character.len_utf8())
            .take_while(|end| columns(&rest[..*end]) <= width)
            .last()
            .unwrap_or_else(|| rest.chars().next().map_or(rest.len(), char::len_utf8));
        let mut line = &rest[..end];
        if end < rest.len()
            && !rest[end..].starts_with(' ')
            && let Some(space) = line.rfind(' ').filter(|space| *space > 0)
        {
            line = &line[..space];
        }
        rest = rest[line.len()..].trim_start_matches(' ');
        let line = line.trim_end_matches(' ');
        if !line.is_empty() {
            lines.push(line.to_owned());
        }
    }
    lines
}
```

In the tests, after `counts_wide_characters_as_two_columns`:

```rust
    #[test]
    fn breaks_at_the_last_space_within_the_width() {
        assert_eq!(
            wrap("lmx:console, lmx:git, lmx:go", 14),
            ["lmx:console,", "lmx:git,", "lmx:go"]
        );
        assert_eq!(wrap("a lmx:console b", 13), ["a lmx:console", "b"]);
        assert_eq!(wrap("aaaa  bbbb", 5), ["aaaa", "bbbb"]);
    }

    #[test]
    fn breaks_long_words_without_splitting_characters() {
        assert_eq!(wrap(&"界".repeat(5), 5), ["界界", "界界", "界"]);
        assert_eq!(wrap("/workspace/project", 10), ["/workspace", "/project"]);
    }
```

After `keeps_continuation_lines_in_the_value_column`:

```rust
    #[test]
    fn measures_joined_emoji_as_one_wide_character() {
        let developer = "👨\u{200d}💻";
        assert_eq!(columns(developer), 2);
        assert_eq!(
            wrap(&format!("{developer} {developer}"), 5),
            [format!("{developer} {developer}")]
        );
    }
```

At the end of the tests:

```rust
    #[test]
    fn keeps_short_text_on_one_line() {
        assert_eq!(wrap("none", 67), ["none"]);
        assert!(wrap("", 67).is_empty());
    }
```

**Step 4: Add the palette**

Create `crates/lmx/src/palette.rs`. `plain` and `colored` exist only for tests; the command decides with `detect`:

```rust
//! Colors of the guest's text: Catppuccin Mocha, the palette of the platform prompt.

use std::{
    env,
    io::{self, IsTerminal},
};

/// One palette color as 24-bit RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Color(u8, u8, u8);

/// Mocha `blue`: commands and the logo.
pub(crate) const BLUE: Color = Color(137, 180, 250);
/// Mocha `mauve`: the second half of the logo.
pub(crate) const MAUVE: Color = Color(203, 166, 247);
/// Mocha `subtext0`: secondary values.
pub(crate) const SUBTEXT: Color = Color(166, 173, 200);
/// Mocha `overlay1`: labels.
pub(crate) const MUTED: Color = Color(127, 132, 156);
/// Mocha `green`: writable shared folders.
pub(crate) const GREEN: Color = Color(166, 227, 161);
/// Mocha `peach`: read-only shared folders.
pub(crate) const PEACH: Color = Color(250, 179, 135);
/// Mocha `yellow`: warnings.
pub(crate) const YELLOW: Color = Color(249, 226, 175);

/// Whether text is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint {
    /// `true` when escape sequences are written.
    enabled: bool,
}

impl Paint {
    /// Colors only a terminal that is not `dumb`, and never when `NO_COLOR` is set; an empty `TERM`
    /// counts as `dumb`, as in the platform prompt.
    pub(crate) fn detect() -> Self {
        let terminal = io::stdout().is_terminal();
        let no_color = env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        let dumb = env::var_os("TERM").is_none_or(|term| term.is_empty() || term == "dumb");
        Self {
            enabled: terminal && !no_color && !dumb,
        }
    }

    /// Plain text, for tests.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self { enabled: false }
    }

    /// Colored text, for tests.
    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self { enabled: true }
    }

    /// `text` in `color`.
    pub(crate) fn color(self, color: Color, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            let Color(red, green, blue) = color;
            format!("\x1b[38;2;{red};{green};{blue}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// `text` in bold.
    pub(crate) fn bold(self, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_escapes() {
        assert_eq!(Paint::plain().color(BLUE, "lmx"), "lmx");
        assert_eq!(Paint::plain().bold("dev-box"), "dev-box");
    }

    #[test]
    fn colored_text_uses_true_color() {
        assert_eq!(
            Paint::colored().color(BLUE, "lmx"),
            "\x1b[38;2;137;180;250mlmx\x1b[0m"
        );
        assert_eq!(Paint::colored().bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(Paint::colored().color(YELLOW, ""), "");
    }
}
```

**Step 5: Locate memory information**

In `crates/lmx/src/system.rs`, replace the `lmx_facts` import:

```rust
use lmx_facts::{disk::STORE_PATH, generations::GenerationPaths, mounts::MOUNTINFO_PATH};
```

with:

```rust
use lmx_facts::{
    disk::STORE_PATH, generations::GenerationPaths, machine::MEMINFO_PATH, mounts::MOUNTINFO_PATH,
};
```

After `mountinfo`:

```rust
    /// Memory information.
    pub(crate) fn meminfo(&self) -> PathBuf {
        self.root.join(MEMINFO_PATH.trim_start_matches('/'))
    }
```

**Step 6: Write the welcome**

Create `crates/lmx/src/welcome.rs`. The golden test `shows_a_healthy_vm` pins the layout of `welcome.sh`;
`wraps_long_values_within_eighty_columns` pins the width limit:

```rust
//! `lmx welcome`: the summary an interactive shell shows when it starts.
//!
//! The layout follows the platform's former shell welcome: logo, VM, resources, modules, shared
//! folders, warnings and the next commands. Labels take 11 columns after a two-column indent, and
//! values wrap so no line is wider than 80 columns. Facts are read first, so rendering is a pure
//! function of them.

use std::{io, process::ExitCode};

use lmx_facts::{
    disk, machine,
    mounts::{self, Mount},
    units,
};
use lmx_model::DiskUsage;

use crate::{
    format::gibibytes,
    help,
    layout::{columns, wrap},
    output,
    palette::{self, Color, Paint},
    system::System,
};

/// Columns of the label after the indent.
const LABEL: usize = 11;
/// Columns left for a value: 80 minus the indent and the label.
const VALUE: usize = 67;
/// Widest shared-folder target before it wraps, leaving room for the mode.
const TARGET: usize = 63;
/// Columns of a warning after the indent.
const WARNING: usize = 78;
/// Columns of each logo line drawn in blue; the rest is mauve.
const LOGO_SPLIT: usize = 36;
/// Warning threshold when the configuration, and with it the platform policy, is unreadable.
const DEFAULT_MINIMUM_PERCENT: u8 = 10;

/// The LimaNix logo.
const LOGO: [&str; 8] = [
    "888      d8b                        888b    888 d8b",
    "888      Y8P                        8888b   888 Y8P",
    "888                                 88888b  888",
    "888      888 88888b.d88b.   8888b.  888Y88b 888 888 888  888",
    r#"888      888 888 "888 "88b     "88b 888 Y88b888 888 `Y8bd8P'"#,
    "888      888 888  888  888 .d888888 888  Y88888 888   X88K",
    r#"888      888 888  888  888 888  888 888   Y8888 888 .d8""8b."#,
    r#"88888888 888 888  888  888 "Y888888 888    Y888 888 888  888"#,
];

/// Everything the welcome shows.
#[derive(Debug)]
struct Facts {
    /// VM name.
    name: String,
    /// Operating system and architecture, such as `NixOS 26.05, arm64`.
    system: String,
    /// Selected catalog modules, or `none`.
    modules: String,
    /// Free share below which the disk warning appears, in percent.
    minimum_percent: u8,
    /// Processors, when readable.
    cpus: Option<usize>,
    /// Total memory in bytes, when readable.
    memory: Option<u64>,
    /// Guest disk usage, when readable.
    disk: Option<DiskUsage>,
    /// Shared folders, or `None` when the mount table is unreadable.
    mounts: Option<Vec<Mount>>,
    /// Failed units; empty when there are none or they cannot be listed.
    failed_units: Vec<String>,
}

/// Runs `lmx welcome`; fails when the shared folders cannot be listed, as the shell welcome did.
pub(crate) fn run(system: &System) -> io::Result<ExitCode> {
    let facts = collect(system);
    output::write_text(&render(&facts, Paint::detect()))?;
    Ok(output::page_status(facts.mounts.is_some()))
}

/// Reads the facts the welcome shows; each one is best effort.
fn collect(system: &System) -> Facts {
    let (name, os, modules, minimum_percent) = match &system.config {
        Ok(config) => (
            config.vm.name.clone(),
            format!("{}, {}", config.vm.system, config.vm.arch),
            help::modules(config),
            config.disk.minimum_percent,
        ),
        Err(_) => (
            "unknown".to_owned(),
            "workspace metadata cannot be read".to_owned(),
            "unknown".to_owned(),
            DEFAULT_MINIMUM_PERCENT,
        ),
    };
    Facts {
        name,
        system: os,
        modules,
        minimum_percent,
        cpus: machine::cpus().ok(),
        memory: machine::memory(&system.meminfo()).ok(),
        disk: disk::usage(&system.store()).ok(),
        mounts: mounts::shared(&system.mountinfo()).ok(),
        failed_units: units::failed(&system.systemctl()).unwrap_or_default(),
    }
}

/// Renders the welcome.
fn render(facts: &Facts, paint: Paint) -> String {
    let mut text = String::from("\n");
    for line in LOGO {
        let (left, right) = line.split_at(LOGO_SPLIT.min(line.len()));
        text.push_str(&format!(
            "  {}{}\n",
            paint.color(palette::BLUE, left),
            paint.color(palette::MAUVE, right)
        ));
    }
    text.push('\n');

    vm(&mut text, facts, paint);
    text.push('\n');

    let resources = resources(facts);
    if !resources.is_empty() {
        row(&mut text, paint, "Resources", &resources, None);
        text.push('\n');
    }

    row(&mut text, paint, "Modules", &facts.modules, None);
    text.push('\n');

    shared(&mut text, facts, paint);
    text.push('\n');

    if let Some(warning) = disk_warning(facts) {
        warn(&mut text, paint, &warning);
    }
    if !facts.failed_units.is_empty() {
        let units = facts.failed_units.join(", ");
        warn(
            &mut text,
            paint,
            &format!("▲ Failed: {units}. Run lmx info for details."),
        );
    }

    text.push_str(&format!(
        "  {}{}{}{}{}{}\n\n",
        paint.color(palette::BLUE, "lmx help"),
        paint.color(palette::MUTED, " for commands, "),
        paint.color(palette::BLUE, "lmx info"),
        paint.color(palette::MUTED, " for details, "),
        paint.color(palette::BLUE, "exit"),
        paint.color(palette::MUTED, " to return to the Mac.")
    ));
    text
}

/// The label column, muted.
fn label(paint: Paint, label: &str) -> String {
    paint.color(palette::MUTED, &format!("{label:<LABEL$}"))
}

/// Writes `label` and `value`; further lines of the value keep the value column.
fn row(text: &mut String, paint: Paint, name: &str, value: &str, color: Option<Color>) {
    let mut name = name;
    for line in wrap(value, VALUE) {
        let line = color.map_or_else(|| line.clone(), |color| paint.color(color, &line));
        text.push_str(&format!("  {}{line}\n", label(paint, name)));
        name = "";
    }
}

/// The VM name in bold and its system, on one line when they fit.
fn vm(text: &mut String, facts: &Facts, paint: Paint) {
    let system = format!("({})", facts.system);
    if columns(&facts.name) + 1 + columns(&system) <= VALUE {
        text.push_str(&format!(
            "  {}{} {}\n",
            label(paint, "VM"),
            paint.bold(&facts.name),
            paint.color(palette::SUBTEXT, &system)
        ));
    } else {
        row(text, paint, "VM", &facts.name, None);
        row(text, paint, "", &system, Some(palette::SUBTEXT));
    }
}

/// Processors, memory and free disk; a part that cannot be read is left out.
fn resources(facts: &Facts) -> String {
    let mut parts = Vec::new();
    match facts.cpus {
        Some(1) => parts.push("1 CPU".to_owned()),
        Some(count) => parts.push(format!("{count} CPUs")),
        None => {}
    }
    if let Some(memory) = facts.memory {
        parts.push(format!("{} memory", gibibytes(memory)));
    }
    if let Some(disk) = facts.disk {
        parts.push(format!("{} disk free", gibibytes(disk.available_bytes)));
    }
    parts.join(", ")
}

/// Shared folders with their mode, aligned after the longest target.
fn shared(text: &mut String, facts: &Facts, paint: Paint) {
    let Some(mounts) = &facts.mounts else {
        row(text, paint, "Shared", "unavailable; run lmx info", None);
        return;
    };
    if mounts.is_empty() {
        row(text, paint, "Shared", "(none)", None);
        return;
    }
    let width = mounts
        .iter()
        .map(|mount| columns(&mount.target))
        .max()
        .unwrap_or_default()
        .min(TARGET);
    let mut name = "Shared";
    for mount in mounts {
        let (mode, color) = if mount.read_only {
            ("ro", palette::PEACH)
        } else {
            ("rw", palette::GREEN)
        };
        for (index, line) in wrap(&mount.target, width).into_iter().enumerate() {
            if index == 0 {
                let padding = width.saturating_sub(columns(&line));
                text.push_str(&format!(
                    "  {}{line}{:padding$}  {}\n",
                    label(paint, name),
                    "",
                    paint.color(color, mode)
                ));
                name = "";
            } else {
                text.push_str(&format!("  {:LABEL$}{line}\n", ""));
            }
        }
    }
}

/// The disk warning when free space or free inodes are below the platform minimum.
fn disk_warning(facts: &Facts) -> Option<String> {
    let disk = facts.disk?;
    let minimum = u128::from(facts.minimum_percent);
    let short: Vec<String> = [
        (disk.available_bytes, disk.bytes, "space"),
        (disk.free_inodes, disk.inodes, "inodes"),
    ]
    .into_iter()
    .filter(|(free, total, _)| *total > 0 && u128::from(*free) * 100 < u128::from(*total) * minimum)
    .map(|(free, total, what)| {
        format!(
            "{}% of {what} free",
            u128::from(free) * 100 / u128::from(total)
        )
    })
    .collect();
    (!short.is_empty()).then(|| {
        format!(
            "▲ Guest disk nearly full: {}. Run lmx info for details.",
            short.join(" and ")
        )
    })
}

/// Writes a warning in yellow, followed by a blank line.
fn warn(text: &mut String, paint: Paint, warning: &str) {
    for line in wrap(warning, WARNING) {
        text.push_str(&format!("  {}\n", paint.color(palette::YELLOW, &line)));
    }
    text.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes in one kibibyte.
    const KIB: u64 = 1024;

    /// A healthy VM with two shared folders.
    fn facts() -> Facts {
        Facts {
            name: "dev-box".into(),
            system: "NixOS 26.05, arm64".into(),
            modules: "none".into(),
            minimum_percent: 10,
            cpus: Some(4),
            memory: Some(7_969_124 * KIB),
            disk: Some(DiskUsage {
                bytes: 103_081_248 * KIB,
                free_bytes: 41_846_681 * KIB,
                available_bytes: 39_800_000 * KIB,
                inodes: 1_000_000,
                free_inodes: 500_000,
            }),
            mounts: Some(vec![
                Mount {
                    target: "/home/dev".into(),
                    fs_type: "virtiofs".into(),
                    read_only: false,
                },
                Mount {
                    target: "/mnt/limanix".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true,
                },
            ]),
            failed_units: vec![],
        }
    }

    /// The text after the logo.
    fn body(text: &str) -> &str {
        let logo_end = text.find(LOGO[7]).expect("logo") + LOGO[7].len() + 1;
        &text[logo_end..]
    }

    #[test]
    fn shows_a_healthy_vm() {
        let text = render(&facts(), Paint::plain());
        assert!(text.starts_with("\n  888      d8b"));
        assert_eq!(
            body(&text),
            "\n  VM         dev-box (NixOS 26.05, arm64)\n\
             \n  Resources  4 CPUs, 7.6 GiB memory, 38 GiB disk free\n\
             \n  Modules    none\n\
             \n  Shared     /home/dev     rw\n             /mnt/limanix  ro\n\
             \n  lmx help for commands, lmx info for details, exit to return to the Mac.\n\n"
        );
    }

    #[test]
    fn warns_about_a_nearly_full_disk_and_failed_units() {
        let mut facts = facts();
        facts.disk = facts.disk.map(|disk| DiskUsage {
            free_inodes: 40_000,
            ..disk
        });
        facts.failed_units = vec!["limanix-store-guard.service".into()];
        let text = render(&facts, Paint::plain());
        assert!(text.contains(
            "  ▲ Guest disk nearly full: 4% of inodes free. Run lmx info for details.\n\n"
        ));
        assert!(
            text.contains("  ▲ Failed: limanix-store-guard.service. Run lmx info for details.\n\n")
        );
    }

    #[test]
    fn explains_missing_shared_folders() {
        let mut facts = facts();
        facts.mounts = Some(vec![]);
        assert!(render(&facts, Paint::plain()).contains("  Shared     (none)\n"));
        facts.mounts = None;
        assert!(
            render(&facts, Paint::plain()).contains("  Shared     unavailable; run lmx info\n")
        );
    }

    #[test]
    fn leaves_out_resources_that_cannot_be_read() {
        let mut facts = facts();
        facts.cpus = None;
        facts.memory = None;
        facts.disk = None;
        let text = render(&facts, Paint::plain());
        assert!(!text.contains("Resources"));
        assert!(text.contains("  Modules    none\n"));
    }

    #[test]
    fn wraps_long_values_within_eighty_columns() {
        let mut facts = facts();
        facts.name = "long-vm-name-".repeat(10);
        facts.modules = format!("{}lmx:git", "lmx:console, ".repeat(12));
        facts.mounts = Some(vec![
            Mount {
                target: format!("/workspace/{}", "project-".repeat(16)),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
            Mount {
                target: format!("/workspace/{}", "界".repeat(50)),
                fs_type: "9p".into(),
                read_only: true,
            },
            Mount {
                target: "/workspace/a $literal 'quote'".into(),
                fs_type: "virtiofs".into(),
                read_only: false,
            },
        ]);
        facts.failed_units = vec![
            format!("{}a.service", "unit-".repeat(12)),
            format!("{}b.service", "unit-".repeat(12)),
        ];
        let text = render(&facts, Paint::plain());
        assert!(text.contains("'quote'"));
        for line in text.lines() {
            assert!(columns(line) <= 80, "{} columns: {line:?}", columns(line));
            assert_eq!(line.trim_end(), line, "trailing spaces: {line:?}");
        }
    }

    #[test]
    fn colors_only_when_asked() {
        assert!(!render(&facts(), Paint::plain()).contains('\x1b'));
        let text = render(&facts(), Paint::colored());
        assert!(text.contains("\x1b[1mdev-box\x1b[0m"));
        assert!(text.contains("\x1b[38;2;250;179;135mro\x1b[0m"));
    }
}
```

**Step 7: Add `lmx welcome` to the command line**

In `crates/lmx/src/cli.rs`, after the `Info` variant:

```rust
    /// Show the workspace welcome again.
    Welcome,
```

In `crates/lmx/src/main.rs`, after `mod output;`:

```rust
mod palette;
```

After `mod version;`:

```rust
mod welcome;
```

In `lmx`, after the info arm:

```rust
        Some(Command::Welcome) => welcome::run(&System::from_environment()),
```

**Step 8: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 42 tests.

**Step 9: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/src crates/lmx/tests/cli.rs
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add lmx welcome"
```

---

## Task 9: Named sessions (`limanix-session`, `lmx session`)

`limanix shell NAME --session PROJECT` runs `limanix-session PROJECT` in the guest. The platform will install `lmx`
under that name (M1c), so `main` starts dispatching on the file name it was started with. `limanix-session` takes
exactly one nonempty argument and passes it to the provider unchanged, even when it looks like an option; clap would
treat `--help` as a flag, so this name is parsed by hand. The provider replaces the process (`exec`), so it owns the
terminal, the streams and the exit status.

**Files:**
- Create: `crates/lmx/src/process.rs`, `crates/lmx/src/session.rs`
- Modify: `crates/lmx/src/output.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`
- Test: `crates/lmx/tests/cli.rs`

**Step 1: Write the failing tests**

In `crates/lmx/tests/cli.rs`, replace the `os::unix::fs` import:

```rust
    os::unix::fs::PermissionsExt,
```

with:

```rust
    os::unix::fs::{PermissionsExt, symlink},
```

After `Guest::path`, add a script helper and a way to change the session configuration:

```rust
    /// Writes an executable shell script with `body` to `relative` and returns its path.
    fn script(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.root.path().join(relative);
        self.write(relative, &format!("#!/bin/sh\n{body}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make script executable");
        path
    }

    /// Replaces the session part of the configuration.
    fn set_session(&self, session: Value) {
        let mut config: Value =
            serde_json::from_slice(&fs::read(self.config()).expect("read configuration"))
                .expect("configuration JSON");
        config["session"] = session;
        self.write("etc/lmx/config.json", &config.to_string());
    }
```

Replace `Guest::command`; `alias` starts the binary through a link, as the platform's other names will:

```rust
    /// Prepares `lmx` with the tree as its system root.
    fn command(&self, args: &[&str], config: &Path) -> Command {
        self.command_as(env!("CARGO_BIN_EXE_lmx"), args, config)
    }

    /// Prepares the binary under one of its other names, through a link named `name`.
    fn alias(&self, name: &str, args: &[&str]) -> Command {
        let link = self.root.path().join("aliases").join(name);
        if !link.exists() {
            fs::create_dir_all(link.parent().expect("link has a parent")).expect("create aliases");
            symlink(env!("CARGO_BIN_EXE_lmx"), &link).expect("link the binary");
        }
        self.command_as(link, args, &self.config())
    }

    /// Prepares `program` with the tree as its system root.
    fn command_as(&self, program: impl AsRef<Path>, args: &[&str], config: &Path) -> Command {
        let mut command = Command::new(program.as_ref());
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", config)
            .env("PATH", self.root.path().join("empty"));
        command
    }
```

Before `a_closed_standard_output_ends_quietly`, add:

```rust
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
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli session`

Expected: FAIL: under the name `limanix-session` the binary still parses its arguments as `lmx`.

**Step 3: Report usage errors**

In `crates/lmx/src/output.rs`, after `FAILURE`:

```rust
/// Exit status of a usage error.
const USAGE: u8 = 2;
```

At the end of the file:

```rust
/// Reports a usage error.
pub(crate) fn usage(usage: &str) -> ExitCode {
    eprintln!("{usage}");
    ExitCode::from(USAGE)
}
```

**Step 4: Replace the process**

Create `crates/lmx/src/process.rs`. A program that cannot start exits 127 or 126, as in a shell:

```rust
//! Replacing `lmx` with another program.

use std::{
    io,
    os::unix::process::CommandExt,
    process::{Command, ExitCode},
};

/// Exit status when the program to run does not exist, as in a shell.
const NOT_FOUND: u8 = 127;
/// Exit status when the program exists but cannot be run, as in a shell.
const NOT_RUNNABLE: u8 = 126;

/// Replaces this process with `command`, which then owns the streams and the exit status.
///
/// Returns only when the program cannot be started.
pub(crate) fn replace(command: &mut Command) -> ExitCode {
    let error = command.exec();
    eprintln!(
        "lmx: cannot run {}: {error}",
        command.get_program().to_string_lossy()
    );
    ExitCode::from(if error.kind() == io::ErrorKind::NotFound {
        NOT_FOUND
    } else {
        NOT_RUNNABLE
    })
}
```

**Step 5: Open sessions**

Create `crates/lmx/src/session.rs`:

```rust
//! `limanix-session` and `lmx session`: open a named session with the selected provider.
//!
//! `limanix shell NAME --session PROJECT` runs `limanix-session PROJECT` in the guest. The name is
//! passed to the provider unchanged, even when it looks like an option, and the provider then owns
//! the terminal and the exit status.

use std::{ffi::OsStr, process::Command, process::ExitCode};

use lmx_model::Config;

use crate::{output, process};

/// Usage of `limanix-session`.
pub(crate) const USAGE: &str = "Usage: limanix-session NAME (one nonempty session name)";

/// Exit status when no provider is configured, as for a missing command in a shell.
const NO_PROVIDER: u8 = 127;

/// Opens session `name` with the configured provider.
pub(crate) fn run(config: &Result<Config, String>, name: &OsStr) -> ExitCode {
    if name.is_empty() {
        return output::usage(USAGE);
    }
    let config = match config {
        Ok(config) => config,
        Err(message) => {
            eprintln!("lmx: {message}");
            return ExitCode::from(output::FAILURE);
        }
    };
    // The platform rendered a missing provider as an empty command.
    let Some(provider) = config
        .session
        .command
        .as_deref()
        .filter(|command| !command.is_empty())
    else {
        eprint!("{}", missing_provider(&config.session.providers));
        return ExitCode::from(NO_PROVIDER);
    };
    process::replace(Command::new(provider).arg(name))
}

/// Explains how to select a provider when none is configured.
fn missing_provider(providers: &[String]) -> String {
    let mut text = String::from("No session provider is configured in this VM.\n");
    if !providers.is_empty() {
        text.push_str(&format!(
            "Select a session provider in nixos.modules: {}\n",
            providers.join(", ")
        ));
        text.push_str(
            "Apply the configuration with limanix update --config <file> before reconnecting.\n",
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_providers_from_the_catalog() {
        let text = missing_provider(&["lmx:tmux".into(), "third-party:sessions".into()]);
        assert!(text.starts_with("No session provider is configured in this VM.\n"));
        assert!(text.contains("nixos.modules: lmx:tmux, third-party:sessions\n"));
        assert!(text.contains("limanix update"));
    }

    #[test]
    fn invents_no_suggestions() {
        assert_eq!(
            missing_provider(&[]),
            "No session provider is configured in this VM.\n"
        );
    }
}
```

**Step 6: Add `lmx session` to the command line**

In `crates/lmx/src/cli.rs`, replace the module documentation and imports:

```rust
//! Command-line interface of `lmx`.
//!
//! The other names of the binary keep the syntax of the commands they replaced and are parsed in
//! `main`; `lmx`, `lmx --help` and `lmx -h` print the guest help page instead of the generated one.

use std::ffi::OsString;

use clap::{Arg, ArgAction, Args, Parser, Subcommand};
```

After the `Version` variant:

```rust
    /// Open a named session with the selected provider; also installed as limanix-session.
    Session(SessionArgs),
```

Before `OutputArgs`:

```rust
/// Arguments of `lmx session`.
#[derive(Debug, Args)]
pub(crate) struct SessionArgs {
    /// Session name, passed to the provider unchanged; put -- before a name that starts with -.
    pub(crate) name: OsString,
}
```

**Step 7: Dispatch on the program name**

In `crates/lmx/src/main.rs`, after `mod palette;`:

```rust
mod process;
mod session;
```

Replace the `std` import:

```rust
use std::{env, ffi::OsString, io, iter, process::ExitCode};
```

with:

```rust
use std::{
    env,
    ffi::{OsStr, OsString},
    io, iter,
    path::Path,
    process::ExitCode,
};
```

In `main`, replace the first lines:

```rust
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();

    let result = lmx(arguments);
```

with:

```rust
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let arguments: Vec<OsString> = arguments.collect();

    let result = match Path::new(&program).file_name().and_then(OsStr::to_str) {
        Some("limanix-session") => Ok(match arguments.as_slice() {
            [name] => session::run(&System::from_environment().config, name),
            _ => output::usage(session::USAGE),
        }),
        _ => lmx(arguments),
    };
```

In `lmx`, after the version arm:

```rust
        Some(Command::Session(args)) => {
            Ok(session::run(&System::from_environment().config, &args.name))
        }
```

**Step 8: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 48 tests.

**Step 9: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/src crates/lmx/tests/cli.rs
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Open named sessions as limanix-session"
```

---

## Task 10: The Mac clipboard (`pbcopy`, `pbpaste`, `lmx clipboard`)

The terminal on the Mac carries the clipboard in OSC 52 sequences. `pbcopy` writes `ESC ] 52 ; c ; <base64> BEL` to
`/dev/tty`. `pbpaste` writes the request `ESC ] 52 ; c ; ? BEL`, then reads the reply from `/dev/tty` without echo or
line buffering for up to 10 seconds; the terminal may ask the person for permission first. Inside tmux, tmux owns the
terminal: `pbcopy` hands standard input to `tmux load-buffer -w -`, and `pbpaste` asks tmux with `refresh-client -l`,
waits for a new buffer and prints it by name with `tmux save-buffer -b NAME -`.

A reply ends with BEL or with the two bytes `ESC \`. Stopping at the `ESC` alone would leave the `\` for the shell to
read once the terminal mode is restored. The reply is scanned again only when a chunk holds one of those bytes, which
base64 never contains, so a large clipboard is read in linear time.

Three test hooks join `LMX_CONFIG` and `LMX_SYSTEM_ROOT`: `LMX_TTY_IN` and `LMX_TTY_OUT` replace `/dev/tty`, and
`LMX_PASTE_TIMEOUT` shortens the wait. A file that is not a terminal keeps its mode, and `poll` ends the wait at its
end. On macOS `poll` reports `/dev/tty` as invalid, so a read is attempted only after `poll` reports input; the guest is
Linux, where `/dev/tty` polls normally.

**Files:**
- Create: `crates/lmx/src/clipboard.rs`
- Modify: `Cargo.toml`, `crates/lmx/Cargo.toml`, `crates/lmx/src/output.rs`, `crates/lmx/src/system.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`
- Test: `crates/lmx/tests/cli.rs`

**Step 1: Add `base64` and the terminal features of rustix**

In the root `Cargo.toml`, after `lmx-model`:

```toml
base64        = "0.23.1"
```

In `crates/lmx/Cargo.toml`, before `clap`:

```toml
base64        = { workspace = true }
```

After `lmx-model`:

```toml
rustix        = { workspace = true, features = ["event", "termios"] }
```

**Step 2: Write the failing tests**

In `crates/lmx/tests/cli.rs`, replace the `std` import:

```rust
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{self, Command, Output},
    sync::OnceLock,
};
```

with:

```rust
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{self, Command, Output, Stdio},
    sync::OnceLock,
};
```

Keep the hooks and tmux of the test runner away from `lmx` in `Guest::command_as`:

```rust
    /// Prepares `program` with the tree as its system root, outside any tmux session.
    ///
    /// The terminal is a file that does not exist, so a test reaches the terminal running the tests
    /// only if it names one.
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
```

After `answer`, add a fake tmux that logs its calls, a way to run inside it, and a runner that feeds standard input:

```rust
/// Installs a fake `tmux` that logs its calls and returns the directory holding it.
///
/// `refresh-client` creates a new buffer only when the terminal `answers`.
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

/// Prepares `command` to run in a tmux session whose `tmux` is the fake one in `bin`.
fn in_tmux<'a>(command: &'a mut Command, bin: &Path) -> &'a mut Command {
    command
        .env("TMUX", "/tmp/tmux-501/default,1,0")
        .env("PATH", bin)
}

/// Runs `command` with `input` on standard input.
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
```

Before `a_closed_standard_output_ends_quietly`, add:

```rust
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
```

**Step 3: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli pb`

Expected: FAIL: under the names `pbcopy` and `pbpaste` the binary still parses its arguments as `lmx`.

**Step 4: Write bytes to standard output**

Clipboard contents need not be UTF-8. In `crates/lmx/src/output.rs`, replace `write_text`:

```rust
/// Writes text for people to standard output, returning write failures instead of panicking
/// like `print!`.
pub(crate) fn write_text(text: &str) -> io::Result<()> {
    write_bytes(text.as_bytes())
}

/// Writes bytes, such as clipboard contents, to standard output.
pub(crate) fn write_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}
```

**Step 5: Share the reader of path hooks**

The clipboard reads its terminal hooks the same way. In `crates/lmx/src/system.rs`, replace:

```rust
/// Path from a test hook, or `None` when the variable is unset or empty.
fn hook(
```

with:

```rust
/// Path from a test hook, or `None` when the variable is unset or empty.
pub(crate) fn hook(
```

**Step 6: Talk to the terminal**

Create `crates/lmx/src/clipboard.rs`:

```rust
//! `pbcopy`, `pbpaste` and `lmx clipboard`: the Mac clipboard through the terminal.
//!
//! The terminal on the Mac carries the clipboard in OSC 52 escape sequences and must allow them in
//! its settings. Inside tmux, tmux owns the terminal and passes its buffers to the attached client,
//! so both commands hand over to tmux.

use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::Errno,
    termios::{self, LocalModes, OptionalActions, SpecialCodeIndex, Termios},
};

use crate::{output, process, system};

/// Usage of `pbcopy`.
pub(crate) const COPY_USAGE: &str = "Usage: pbcopy < FILE";
/// Usage of `pbpaste`.
pub(crate) const PASTE_USAGE: &str = "Usage: pbpaste";

/// Terminal of the calling session.
const TTY: &str = "/dev/tty";
/// How long `pbpaste` waits for the terminal, which may first ask for permission.
const PASTE_TIMEOUT: Duration = Duration::from_secs(10);
/// Interval between checks for a new tmux buffer.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// OSC 52 request for the clipboard contents.
const PASTE_REQUEST: &[u8] = b"\x1b]52;c;?\x07";
/// Start of an OSC 52 reply.
const REPLY_START: &[u8] = b"\x1b]52;";

/// Copies standard input to the Mac clipboard.
pub(crate) fn copy() -> io::Result<ExitCode> {
    if let Some(tmux) = tmux() {
        return Ok(process::replace(Command::new(tmux).args([
            "load-buffer",
            "-w",
            "-",
        ])));
    }
    let mut data = Vec::new();
    io::stdin().read_to_end(&mut data)?;
    let written = OpenOptions::new()
        .write(true)
        .open(terminal("LMX_TTY_OUT"))
        .and_then(|mut terminal| terminal.write_all(copy_sequence(&data).as_bytes()));
    if written.is_err() {
        eprintln!("pbcopy needs the terminal of a limanix shell session.");
        return Ok(ExitCode::from(output::FAILURE));
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints the Mac clipboard.
pub(crate) fn paste() -> io::Result<ExitCode> {
    let timeout = paste_timeout();
    if let Some(tmux) = tmux() {
        return paste_through_tmux(&tmux, timeout);
    }
    let opened = File::open(terminal("LMX_TTY_IN")).and_then(|reader| {
        let writer = OpenOptions::new()
            .write(true)
            .open(terminal("LMX_TTY_OUT"))?;
        Ok((reader, writer))
    });
    let Ok((reader, mut writer)) = opened else {
        eprintln!("pbpaste needs the terminal of a limanix shell session.");
        return Ok(ExitCode::from(output::FAILURE));
    };
    let reply = {
        // The reply arrives as terminal input without a newline: read it raw and without echo.
        let _raw = RawMode::enter(&reader);
        writer.write_all(PASTE_REQUEST)?;
        writer.flush()?;
        read_reply(&reader, timeout)?
    };
    match decode_reply(&reply) {
        Some(data) => {
            output::write_bytes(&data)?;
            Ok(ExitCode::SUCCESS)
        }
        None => Ok(unanswered()),
    }
}

/// OSC 52 sequence that sets the clipboard to `data`.
fn copy_sequence(data: &[u8]) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(data))
}

/// Clipboard contents from the first OSC 52 reply in `input`.
///
/// A reply is `ESC ] 52 ; <selection> ; <base64>` ended by BEL or `ESC \`. Keys typed while
/// waiting may surround it. A terminal that refuses reads answers with no data or with `?`.
fn decode_reply(input: &[u8]) -> Option<Vec<u8>> {
    let data = payload(input)?;
    if data.is_empty() || data == b"?" {
        return None;
    }
    STANDARD.decode(data).ok()
}

/// The base64 part of the first complete OSC 52 reply in `input`.
fn payload(input: &[u8]) -> Option<&[u8]> {
    let start = input
        .windows(REPLY_START.len())
        .position(|window| window == REPLY_START)?;
    let reply = &input[start + REPLY_START.len()..];
    // ST is two bytes: a reply ends only with its `\`, which must not be left for the shell.
    let end = (0..reply.len()).find(|&index| match reply[index] {
        b'\x07' => true,
        b'\x1b' => reply.get(index + 1) == Some(&b'\\'),
        _ => false,
    })?;
    let body = &reply[..end];
    Some(&body[body.iter().position(|byte| *byte == b';')? + 1..])
}

/// Reads terminal input until it holds a complete reply, the input ends, or `timeout` passes.
fn read_reply(terminal: &File, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut input = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = Timespec::try_from(remaining).map_err(io::Error::other)?;
        let mut ready = [PollFd::new(terminal, PollFlags::IN)];
        match poll(&mut ready, Some(&wait)) {
            Ok(0) => break,
            Ok(_) => {}
            // Interrupted by a signal: wait for the rest of the time.
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        // Without input, the terminal hung up or cannot be polled, as `/dev/tty` on macOS; a read
        // would block past the timeout.
        if !ready[0].revents().contains(PollFlags::IN) {
            break;
        }
        let count = (&*terminal).read(&mut buffer)?;
        if count == 0 {
            break;
        }
        input.extend_from_slice(&buffer[..count]);
        // Base64 holds neither byte that ends a reply, so only a chunk with one needs a new scan;
        // scanning after every chunk would take quadratic time on a large clipboard.
        let chunk = &buffer[..count];
        if chunk.iter().any(|byte| matches!(byte, b'\x07' | b'\\')) && payload(&input).is_some() {
            break;
        }
    }
    Ok(input)
}

/// Asks tmux for the clipboard and prints the buffer it creates.
fn paste_through_tmux(tmux: &Path, timeout: Duration) -> io::Result<ExitCode> {
    let newest = || {
        Command::new(tmux)
            .args(["list-buffers", "-F", "#{buffer_created} #{buffer_name}"])
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(str::to_owned)
            })
    };
    let before = newest();
    if !Command::new(tmux)
        .args(["refresh-client", "-l"])
        .status()?
        .success()
    {
        return Ok(ExitCode::from(output::FAILURE));
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // The new buffer is printed by name, so a buffer created after it is not printed instead.
        if let Some(created) = newest().filter(|newest| Some(newest) != before.as_ref()) {
            let name = created
                .split_once(' ')
                .map_or(created.as_str(), |(_, name)| name);
            return Ok(process::replace(Command::new(tmux).args([
                "save-buffer",
                "-b",
                name,
                "-",
            ])));
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(unanswered())
}

/// Explains that the terminal did not answer.
fn unanswered() -> ExitCode {
    eprintln!(
        "The terminal did not share its clipboard. Allow clipboard reads in its settings, or paste with Cmd+V."
    );
    ExitCode::from(output::FAILURE)
}

/// tmux from `PATH`, when the caller runs inside a tmux session.
fn tmux() -> Option<PathBuf> {
    env::var_os("TMUX").filter(|value| !value.is_empty())?;
    env::split_paths(&env::var_os("PATH")?)
        .map(|directory| directory.join("tmux"))
        .find(|candidate| {
            candidate.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

/// The terminal device, or the file the test hook `name` points to instead.
fn terminal(name: &str) -> PathBuf {
    system::hook(name).unwrap_or_else(|| PathBuf::from(TTY))
}

/// The paste timeout, or the whole seconds of the `LMX_PASTE_TIMEOUT` test hook.
fn paste_timeout() -> Duration {
    env::var("LMX_PASTE_TIMEOUT")
        .ok()
        .and_then(|seconds| seconds.parse::<u32>().ok())
        .map_or(PASTE_TIMEOUT, |seconds| Duration::from_secs(seconds.into()))
}

/// Terminal mode without echo and line buffering, restored when dropped.
///
/// A file that is not a terminal, such as one a test hook points to, is left alone.
#[derive(Debug)]
struct RawMode<'a> {
    /// Terminal whose mode changed.
    terminal: &'a File,
    /// Mode to restore, when the file is a terminal.
    saved: Option<Termios>,
}

impl<'a> RawMode<'a> {
    /// Turns off echo and line buffering on `terminal`.
    fn enter(terminal: &'a File) -> Self {
        let saved = termios::tcgetattr(terminal).ok();
        if let Some(mode) = &saved {
            let mut raw = mode.clone();
            raw.local_modes
                .remove(LocalModes::ECHO | LocalModes::ICANON);
            // As `cfmakeraw`: a read returns as soon as one byte arrives.
            raw.special_codes[SpecialCodeIndex::VMIN] = 1;
            raw.special_codes[SpecialCodeIndex::VTIME] = 0;
            // A terminal that refuses the change still answers, only echoed.
            let _ = termios::tcsetattr(terminal, OptionalActions::Now, &raw);
        }
        Self { terminal, saved }
    }
}

impl Drop for RawMode<'_> {
    fn drop(&mut self) {
        if let Some(mode) = &self.saved {
            let _ = termios::tcsetattr(self.terminal, OptionalActions::Now, mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_clipboard_for_the_terminal() {
        assert_eq!(copy_sequence(b"hello\n"), "\x1b]52;c;aGVsbG8K\x07");
    }

    #[test]
    fn decodes_replies_ended_by_bel_or_st() {
        assert_eq!(
            decode_reply(b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07").as_deref(),
            Some(&b"hello world"[..])
        );
        assert_eq!(
            decode_reply(b"\x1b]52;c;aGVsbG8=\x1b\\").as_deref(),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn waits_for_the_whole_string_terminator() {
        assert_eq!(payload(b"\x1b]52;c;aGVsbG8=\x1b"), None);
        assert_eq!(payload(b"\x1b]52;c;aGVsbG8=\x1b\\"), Some(&b"aGVsbG8="[..]));
    }

    #[test]
    fn ignores_keys_typed_around_the_reply() {
        assert_eq!(
            decode_reply(b"ls\x1b]52;c;aGVsbG8=\x07\r").as_deref(),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn treats_refusals_and_other_input_as_unanswered() {
        for input in [
            &b""[..],
            b"\x1b]52;c;?\x07",
            b"\x1b]52;c;\x07",
            b"\x1b]52;c;aGVsbG8=",
            b"\x1b]11;rgb:0000/0000/0000\x07",
            b"\x1b]52;c;not base64!\x07",
        ] {
            assert_eq!(decode_reply(input), None, "{input:?}");
        }
    }
}
```

**Step 7: Add `lmx clipboard` to the command line**

In `crates/lmx/src/cli.rs`, before the `Session` variant:

```rust
    /// Use the Mac clipboard through the terminal; also installed as pbcopy and pbpaste.
    #[command(subcommand)]
    Clipboard(ClipboardCommand),
```

Before `SessionArgs`:

```rust
/// Clipboard operations.
#[derive(Debug, Subcommand)]
pub(crate) enum ClipboardCommand {
    /// Copy standard input to the Mac clipboard.
    Copy,
    /// Print the Mac clipboard, if the terminal allows reads.
    Paste,
}
```

**Step 8: Dispatch the clipboard names**

In `crates/lmx/src/main.rs`, after `mod cli;`:

```rust
mod clipboard;
```

Replace the `cli` import:

```rust
    cli::{Cli, Command},
```

with:

```rust
    cli::{Cli, ClipboardCommand, Command},
```

In `main`, before the `limanix-session` arm:

```rust
        Some("pbcopy") if arguments.is_empty() => clipboard::copy(),
        Some("pbcopy") => Ok(output::usage(clipboard::COPY_USAGE)),
        Some("pbpaste") if arguments.is_empty() => clipboard::paste(),
        Some("pbpaste") => Ok(output::usage(clipboard::PASTE_USAGE)),
```

In `lmx`, before the session arm:

```rust
        Some(Command::Clipboard(ClipboardCommand::Copy)) => clipboard::copy(),
        Some(Command::Clipboard(ClipboardCommand::Paste)) => clipboard::paste(),
```

**Step 9: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS, 62 tests.

**Step 10: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add Cargo.toml Cargo.lock crates/lmx
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Use the Mac clipboard as pbcopy and pbpaste"
```

---

## Task 11: Documentation

**Files:**
- Modify: `crates/lmx/src/main.rs`, `README.md`, `ARCHITECTURE.md`, `docs/plans/2026-10-06-guest-owner-design.md`

**Step 1: Document the binary**

In `crates/lmx/src/main.rs`, replace the crate documentation above `#![forbid(unsafe_code)]`. It lists every command,
the other names and all five test hooks:

```rust
//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
//!
//! | Command               | Kind   | Answers or does                                             |
//! |-----------------------|--------|-------------------------------------------------------------|
//! | `lmx help`            | facts  | the workspace and the commands inside the VM and on the Mac |
//! | `lmx info`            | facts  | kernel, guest disk, shared folders and failed units         |
//! | `lmx welcome`         | caller | the summary an interactive shell shows when it starts       |
//! | `lmx status`          | facts  | generations, disk, interfaces and failed units              |
//! | `lmx version`         | facts  | the binary version and the host contract it speaks          |
//! | `lmx clipboard copy`  | caller | copies standard input to the Mac clipboard                  |
//! | `lmx clipboard paste` | caller | prints the Mac clipboard, if the terminal allows reads      |
//! | `lmx session NAME`    | caller | opens a named session with the selected provider            |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon. Caller
//! commands act on the caller's terminal and environment, so only the caller can run them.
//!
//! ## Other names
//!
//! Started under the name of a shell command it replaces, the binary runs that command. The
//! arguments, messages and exit statuses stay those of the replaced command.
//!
//! | Name                   | Runs                  |
//! |------------------------|-----------------------|
//! | `pbcopy`               | `lmx clipboard copy`  |
//! | `pbpaste`              | `lmx clipboard paste` |
//! | `limanix-session NAME` | `lmx session NAME`    |
//!
//! ## Test hooks
//!
//! Environment variables let tests point the binary at prepared files. `sudo` drops all of them by
//! default, so the host never sets them by accident.
//!
//! | Variable            | Replaces                                               |
//! |---------------------|--------------------------------------------------------|
//! | `LMX_CONFIG`        | [`lmx_model::CONFIG_PATH`]                             |
//! | `LMX_SYSTEM_ROOT`   | `/` for generation markers, the store path and `/proc` |
//! | `LMX_TTY_IN`        | `/dev/tty` for reading the terminal's clipboard reply  |
//! | `LMX_TTY_OUT`       | `/dev/tty` for writing clipboard sequences             |
//! | `LMX_PASTE_TIMEOUT` | the 10 seconds `pbpaste` waits for a reply, in seconds |
```

**Step 2: Update the README**

In `README.md`, replace the `## Commands` section:

```markdown
## Commands

| Command               | Answers or does                                                                           |
| --------------------- | ----------------------------------------------------------------------------------------- |
| `lmx help`            | The workspace and the commands inside the VM and on the Mac; also `lmx`, `lmx -h`         |
| `lmx info`            | The kernel, guest disk, shared folders and failed units                                   |
| `lmx welcome`         | The summary an interactive shell prints when it starts                                    |
| `lmx status`          | Desired, built and booted generations; store disk usage; interfaces; failed systemd units |
| `lmx version`         | The binary version and the host contract version                                          |
| `lmx clipboard copy`  | Copies standard input to the Mac clipboard                                                |
| `lmx clipboard paste` | Prints the Mac clipboard, if the terminal allows reads                                    |
| `lmx session NAME`    | Opens a named session with the provider that the selected modules configure               |

Add `--json` to `status` and `version` to answer with the [host contract](docs/contract.md).
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.

Started under the name `pbcopy`, `pbpaste` or `limanix-session`, the binary keeps the arguments, messages and exit statuses of the shell command it replaces.
```

After the bullet about facts in `## Boundaries worth knowing early`:

```markdown
- The clipboard travels through the terminal with OSC 52, or through tmux inside tmux; the terminal on the Mac must allow it.
- Text is colored only on a terminal, never with `NO_COLOR` or `TERM=dumb`.
```

**Step 3: Update the contributor map**

In `ARCHITECTURE.md`, replace the diagram and the paragraph below it:

````markdown
```text
NixOS ──► /etc/lmx/config.json ──► lmx-model::Config
                                        │
person or host ──► lmx (binary) ──► lmx-facts readers ──► statvfs, /proc, uname, ip, systemctl, markers
                         │
                         ├──► lmx-model::Envelope<T> ──► text or JSON on standard output
                         └──► the caller's terminal or tmux, the session provider
```

`lmx-model` holds every value that crosses a boundary: the configuration written by NixOS and the answers read by the host.
`lmx-facts` reads the running system. The `lmx` binary parses the command line, combines facts, and renders them.
Caller commands, the welcome, the clipboard and sessions, depend on the caller's terminal and environment; the welcome also reads facts.
````

In `## Boundaries to preserve`, the `Version` move is done; replace the first bullet:

```markdown
- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract. The `lmx version` answer still lives in the binary and moves to `lmx-model` in M1b.
```

with:

```markdown
- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract.
```

After the last bullet of that section:

```markdown
- Caller commands need the caller's terminal and environment, so they stay in the `lmx` process and never move into a daemon.
- `pbcopy`, `pbpaste` and `limanix-session` keep the syntax of the shell commands they replaced; change them together with the platform.
```

Replace the source map table:

```markdown
| Area              | Responsibility                                                 | Start here                                            |
| ----------------- | -------------------------------------------------------------- | ----------------------------------------------------- |
| Contract types    | Configuration, envelope, error codes, status and version       | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs) |
| Fact readers      | Disk, generations, machine, mounts, network and failed units   | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs) |
| Command line      | Commands, other names, output selection and exit codes         | [`lmx/src/main.rs`](crates/lmx/src/main.rs)           |
| Status            | Collecting facts and rendering them                            | [`lmx/src/status.rs`](crates/lmx/src/status.rs)       |
| Guest pages       | Help, info and the welcome for people in the guest             | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)     |
| Terminal text     | Columns, wrapping and the palette                              | [`lmx/src/layout.rs`](crates/lmx/src/layout.rs)       |
| Caller commands   | The clipboard through the terminal or tmux, and named sessions | [`lmx/src/clipboard.rs`](crates/lmx/src/clipboard.rs) |
| Contract examples | Published answers of each contract version                     | [`contract/v1/`](contract/v1)                         |
```

In `## Add a fact`, before the paragraph about compatible changes:

```markdown
A fact that only a guest page shows, such as `mounts` and `machine`, skips steps 1 and 3.
```

**Step 4: Record M1b in the design**

The design's status line is the entry point the README links. In `docs/plans/2026-10-06-guest-owner-design.md`, replace:

```markdown
Status: design agreed on 2026-10-06. M1a is implemented: repository, contract model, facts,
`lmx status`, release pipeline. See [the M1a plan](2026-10-06-m1a-lmx-foundation.md).
```

with:

```markdown
Status: design agreed on 2026-10-06. M1a is implemented: repository, contract model, facts,
`lmx status`, release pipeline. See [the M1a plan](2026-10-06-m1a-lmx-foundation.md). M1b is
implemented: help, info, welcome, clipboard and sessions in the binary. See
[the M1b plan](2026-10-06-m1b-lmx-user-surface.md).
```

**Step 5: Check the formatting**

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/markdown-fmt`

Expected: PASS. If it fails, run `task --dir <repo> --yes markdown/fix` and review the diff.

Run: `RUSTDOCFLAGS='-D warnings' cargo +1.90.0 doc --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace --no-deps --document-private-items`

Expected: no warnings.

**Step 6: Commit (the user runs it)**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx/src/main.rs README.md ARCHITECTURE.md docs/plans
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Document the user surface and add the M1b plan"
```

---

## Task 12: Final verification and hand-over

**Step 1: Run every CI check in the container**

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-fmt`

Expected: PASS.

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-clippy`

Expected: PASS.

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-test`

Expected: PASS, 100 tests on Linux.

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-docs`

Expected: PASS.

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-audit`

Expected: PASS.

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/markdown-fmt`

Expected: PASS.

**Step 2: Build the release archives**

Run: `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes release/build`

Expected: `dist/lmx-0.1.0-aarch64-linux.tar.gz`, `dist/lmx-0.1.0-x86_64-linux.tar.gz` and their `.sha256` files.

**Step 3: Check the clipboard on a real terminal**

The tests replace `/dev/tty` with files, so they cannot see raw mode or echo. This check drives the static aarch64
binary through a pseudo-terminal on Linux. Save the script as `dist/check-clipboard.py`:

```python
"""Drives pbcopy and pbpaste through a pseudo-terminal, as the terminal on the Mac would."""

import base64
import os
import pty
import select
import sys
import time

LMX = sys.argv[1]
REQUEST = b"\x1b]52;c;?\x07"


def alias(name):
    """Links the binary under one of its other names."""
    path = f"/tmp/{name}"
    if not os.path.lexists(path):
        os.symlink(LMX, path)
    return path


def run(script, reply=(), env=None, limit=20):
    """Runs `script` on a new terminal and answers a paste request with the chunks of `reply`.

    The answer starts 0.3 s after the request, as after a permission prompt, and its chunks arrive
    0.2 s apart, as packets would.
    """
    pid, terminal = pty.fork()
    if pid == 0:
        os.execve("/bin/sh", ["sh", "-c", script], {**os.environ, **(env or {})})
    output, started, pending = b"", time.time(), list(reply)
    while time.time() - started < limit:
        if select.select([terminal], [], [], 0.2)[0]:
            try:
                chunk = os.read(terminal, 65536)
            except OSError:
                break
            if not chunk:
                break
            output += chunk
            if pending and REQUEST in output:
                for index, part in enumerate(pending):
                    time.sleep(0.3 if index == 0 else 0.2)
                    view = memoryview(part)
                    while view:
                        view = view[os.write(terminal, view) :]
                pending = []
        elif os.waitpid(pid, os.WNOHANG)[0]:
            break
    return output.decode(errors="replace"), time.time() - started


def restored(text):
    """Whether `stty -a` after the command shows echo and line buffering again."""
    return "-icanon" not in text and "-echo " not in text


pbpaste, pbcopy = alias("pbpaste"), alias("pbcopy")
# After pbpaste: its status, the terminal mode, then whatever input it left for the shell.
report = """; echo " status=$?"; stty -a; stty -icanon min 0 time 5; printf ' left=['; od -An -c | tr -d ' \\n'; printf ']'"""
for end, reply in (
    ("BEL", [b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07"]),
    ("ST", [b"\x1b]52;c;aGVsbG8=\x1b\\"]),
    ("ST split after ESC", [b"\x1b]52;c;aGVsbG8=\x1b", b"\\"]),
):
    text, _ = run(pbpaste + report, reply=reply)
    pasted = text.split(" status=")[0].replace(REQUEST.decode(), "")
    left = text.split(" left=[")[1].split("]")[0]
    print(
        f"pbpaste, reply ended by {end}: {pasted!r}, echoed: {'aGVsbG8' in text}, "
        f"left for the shell: {left!r}, restored: {restored(text)}"
    )
large = b"x" * (3 << 20)
text, seconds = run(pbpaste + report, reply=[b"\x1b]52;c;" + base64.b64encode(large) + b"\x07"])
pasted = text.split(" status=")[0].replace(REQUEST.decode(), "")
status = text.split(" status=")[1].split()[0]
# Reading the reply in linear time keeps this well under the 10-second paste timeout.
print(f"pbpaste, 4 MiB reply: {len(pasted)} bytes, exit {status}, within 3 s: {seconds < 3}")
text, seconds = run(pbpaste + '; echo " status=$?"; stty -a', env={"LMX_PASTE_TIMEOUT": "2"})
status = text.split(" status=")[1].split()[0]
print(f"pbpaste, no reply: exit {status} after {round(seconds)} s, restored: {restored(text)}")
text, _ = run(f"printf 'copy me' | {pbcopy}" + '; echo " status=$?"')
print(f"pbcopy: {text.strip()!r}")
```

```bash
tar -xzf /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist/lmx-0.1.0-aarch64-linux.tar.gz -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist
docker run --rm -v /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist:/work --entrypoint python3 ghcr.io/mr-chelyshkin/ci/python:3.14.7 /work/check-clipboard.py /work/lmx-0.1.0-aarch64-linux/lmx
```

Expected output:

```text
pbpaste, reply ended by BEL: 'hello world', echoed: False, left for the shell: '', restored: True
pbpaste, reply ended by ST: 'hello', echoed: False, left for the shell: '', restored: True
pbpaste, reply ended by ST split after ESC: 'hello', echoed: False, left for the shell: '', restored: True
pbpaste, 4 MiB reply: 3145728 bytes, exit 0, within 3 s: True
pbpaste, no reply: exit 1 after 2 s, restored: True
pbcopy: '\x1b]52;c;Y29weSBtZQ==\x07 status=0'
```

Remove `dist/` afterwards; it is ignored by Git.

**Step 4: Hand over**

Run `git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx status --short` and give the user the list of changed
files, the test counts and the suggested commits of Tasks 1–11. Do not commit.

---

## Out of scope for M1b

- **M1c, the platform:**
  - install `lmx` from the pinned release and link `pbcopy`, `pbpaste` and `limanix-session` to it, as links rather
    than wrapper scripts, so the binary sees the name and `limanix-session --help` reaches the provider;
  - write `/etc/lmx/config.json` with `vm.system`, `modules`, `disk`, `session.command` and `session.providers`;
  - run `lmx welcome` from `interactiveShellInit` under the same conditions as today;
  - remove `lmx.sh`, `help.sh`, `info.sh`, `welcome.sh`, `pbcopy.sh`, `pbpaste.sh`, `session.sh`, their Nix wiring and
    `/etc/limanix/workspace`, which nothing reads any more; keep the fallback prompt, `programs.bash.promptInit`,
    which lives in the same `workspace.nix`;
  - move the Go tests of those scripts in `client/internal/nixos/{workspace,clipboard,session}_test.go` to the binary
    or drop the cases that `crates/lmx/tests/cli.rs` now covers; `generated_eval_test.go` also asserts packages named
    `pbcopy` and `pbpaste`, which links inside the `lmx` package no longer are;
  - list `lmx status` among the stable guest commands in `client/guides/workspace.md` and `architecture.md`.
- **M1c, the client:** the M1a follow-ups (decoding `lmx status --json`, the fallback when no JSON answer arrives).
- **M2, the store guard:** judge free space by available bytes, as the welcome does. `store-guard.sh` measures free
  blocks including the root reserve, so its threshold and the welcome's warning differ by that reserve.
- **M2, test hooks:** `lmxd` must not read the `LMX_*` hooks. `sudo` protects the CLI by resetting the environment, but
  a service gets no such reset, and the platform applies `/etc/limanix/environment` to every service.
- **M4, status for people:** `lmx status` run by the dev user lists the desired generation, which lives on the host
  mount, as a problem. The people's view (`status --short`) should not render expected permission gaps as problems.
- **Theme (M4):** `palette.rs` keeps Catppuccin Mocha built in until the theme is declared once and rendered by `lmx`.
- **Display of unusual mount points:** `info` and the welcome print a target's control characters, such as a newline
  the kernel escaped as `\012`, as they are. One display helper that escapes them would serve both pages.
- **The first tag:** `v0.1.0` can be tagged once M1b is merged into `main`, because the release workflow refuses tags
  that are not on `main`. The guest's public commands now exist in the binary that M1c switches the platform to.
