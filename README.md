# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

> **The owner of a LimaNix VM from inside: one command for people in the guest and one versioned contract for the host.**

`lmx` runs inside every [LimaNix](https://limanix.dev) guest.
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.

[The problem](#the-problem) · [Commands](#commands) · [Host contract](docs/contract.md) · [Contributor map](ARCHITECTURE.md) · [Design](docs/plans/2026-10-06-guest-owner-design.md)

## The problem

A LimaNix VM is a long-lived development environment, but nothing inside the guest owned it.
The host reached in with literal commands over SSH and parsed their text.
Disk maintenance lived in four places, and the host could not tell which NixOS generation the guest had booted.

`lmx` gives the guest one owner and the host one contract:

```text
host ── ssh ──► sudo lmx <command> --json ──► {"contract": 1, "ok": true, "data": {…}}
person ───────► lmx <command>              ──► text for people
```

## Commands

| Command       | Answers                                                                                   |
| ------------- | ----------------------------------------------------------------------------------------- |
| `lmx status`  | Desired, built and booted generations; store disk usage; interfaces; failed systemd units |
| `lmx version` | The binary version and the host contract version                                          |

Add `--json` to answer with the [host contract](docs/contract.md).
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.

## Boundaries worth knowing early

- `lmx` has no network listener. The host reaches it only through management SSH.
- Facts are read in the caller's process with the caller's privileges and need no daemon.
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
- Release binaries are static musl executables for `aarch64` and `x86_64` Linux.

## Development

Requirements: [Task](https://taskfile.dev/docs/installation) 3.53.1+, Git and Docker.
Tasks run Cargo in the [`ci/rust`](https://github.com/mr-chelyshkin/images) image, so CI and local checks use one toolchain.

| Task                   | Does                                                      |
| ---------------------- | --------------------------------------------------------- |
| `task ci/rust-fmt`     | Checks Rust formatting                                    |
| `task ci/rust-clippy`  | Lints every target and denies warnings                    |
| `task ci/rust-test`    | Runs unit and integration tests                           |
| `task ci/rust-docs`    | Builds API documentation and denies rustdoc warnings      |
| `task ci/rust-audit`   | Scans dependencies for advisories                         |
| `task ci/markdown-fmt` | Checks Markdown formatting                                |
| `task release/build`   | Builds `dist/lmx-<version>-<system>.tar.gz` and checksums |

Pull requests run every task in the table. `task ci` lists the checks.
`task rust/fix` and `task markdown/fix` apply the formatting that the checks expect.

Read the [contributor map](ARCHITECTURE.md) before changing a crate boundary or the host contract.

Licensed under [Apache 2.0](LICENSE).
