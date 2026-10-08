# lmx contributor map

This document is the entry point for contributors and reviewers.
It explains what each crate owns, how the crates connect, and where to begin a change.

For usage, start with the [README](README.md) and the [host contract](docs/contract.md).
Exact contracts live in the Rust source and its module-level documentation.

## Architecture at a glance

```text
NixOS ──► /etc/lmx/config.json ──► lmx-model::Config
                                        │
person or host ──► lmx (binary) ──► lmx-facts readers ──► statvfs, /proc, uname, ip, systemctl, markers
                         │
                         ├──► lmx-model::Envelope<T> ──► text or JSON on standard output
                         ├──► the caller's terminal or tmux, the session provider
                         └──► lmx-ipc ── gRPC, /run/lmx/lmx.sock ──► lmxd (root, systemd)
                                                                      ├──► store guard and reserve
                                                                      ├──► apply, health and finalize
                                                                      └──► Solti tasks ──► nix-store, nixos-rebuild,
                                                                                           nix-env, systemctl, sudo,
                                                                                           systemd-run
```

`lmx-model` holds every value that crosses a boundary: the configuration written by NixOS and the answers read by the host.
`lmx-facts` reads the running system. The `lmx` binary parses the command line, combines facts, and renders them.
`lmxd` owns operations that outlive a caller; `lmx` asks for them through `lmx-ipc` and reports the answer.
Every program `lmxd` runs is a Solti task of a kind under `lmx.limanix.dev/v1`, executed by a private subprocess runner.
Caller commands, the welcome, the clipboard and sessions, depend on the caller's terminal and environment; the welcome also reads facts.

## Boundaries to preserve

- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract.
- `lmx-facts` performs reads only. It never changes the system and never needs a daemon.
- A reader that runs a program splits process I/O from a pure parser; tests cover the parser with fixed output.
- The host contract changes only as described in [Change the host contract](docs/contract.md#change-the-host-contract).
- Configuration comes from NixOS. The binaries never accept configuration from the host at runtime.
- Every crate forbids unsafe Rust with `#![forbid(unsafe_code)]`.
- Caller commands stay in the `lmx` process and never move into a daemon: they need the caller's terminal and environment.
- Owner operations run only in `lmxd`. `lmx` never runs them itself when the daemon is unavailable.
- `lmx-ipc` and `lmx` do not depend on Solti; only `lmxd` does.
- `lmxd` runs programs only as Solti tasks, never with `std::process`: tools by their absolute paths in the configuration, and `/bin/sh` for the roots report, the health check and finalize.
- An apply's state lives in `lmxd`'s memory; after a restart the generation markers are the truth. Finalize runs only in the daemon of the booted system, never in `lmxd --transient`.
- One daemon serves the socket at a time, and two daemons never apply at once: `lmxd` refuses a socket that another daemon still serves.
- `lmxd` writes its journal fields without a prefix: task output carries `LMX_TASK` and `LMX_KIND`, apply events also `LMX_GENERATION`. `lmx logs` depends on these names.
- Colors come from `theme.palette` in the configuration; `lmx` never hard-codes another theme than its Mocha fallback.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
- `pbcopy`, `pbpaste` and `limanix-session` keep the syntax of the shell commands they replaced; change them together with the platform.

## Source map

| Area                | Responsibility                                                                    | Start here                                                                    |
| ------------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Contract types      | Configuration, envelope, error codes, status, apply and version                   | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs)                         |
| Fact readers        | Disk, generations, machine, mounts, network, sockets, journal and failed units    | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs)                         |
| Command line        | Commands, other names, output selection and exit codes                            | [`lmx/src/main.rs`](crates/lmx/src/main.rs)                                   |
| Status              | Collecting facts and rendering them                                               | [`lmx/src/status.rs`](crates/lmx/src/status.rs)                               |
| Diagnostics         | `lmx doctor` and `lmx net check`, and their check records                         | [`lmx/src/doctor.rs`](crates/lmx/src/doctor.rs)                               |
| Task history        | `lmx logs` from the journal fields of `lmxd`                                      | [`lmx/src/logs.rs`](crates/lmx/src/logs.rs)                                   |
| Guest pages         | Help, info and the welcome for people in the guest                                | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)                             |
| Terminal text       | Columns, wrapping and the palette                                                 | [`lmx/src/layout.rs`](crates/lmx/src/layout.rs)                               |
| Caller commands     | The clipboard through the terminal or tmux, and named sessions                    | [`lmx/src/clipboard.rs`](crates/lmx/src/clipboard.rs)                         |
| Owner calls         | `lmx store reserve`, `lmx apply`, and the owner part and the wait of `lmx status` | [`lmx/src/owner.rs`](crates/lmx/src/owner.rs)                                 |
| IPC                 | The `lmx.v1.Owner` protocol and its Unix-socket client                            | [`lmx-ipc/proto/lmx/v1/owner.proto`](crates/lmx-ipc/proto/lmx/v1/owner.proto) |
| Daemon              | Startup, serving the socket, systemd and the stopping order                       | [`lmxd/src/main.rs`](crates/lmxd/src/main.rs)                                 |
| Store domain        | The guard, reserve, conditions and the store tasks                                | [`lmxd/src/store.rs`](crates/lmxd/src/store.rs)                               |
| Apply               | Environment files, reserve and build of a mounted generation, and its followers   | [`lmxd/src/apply.rs`](crates/lmxd/src/apply.rs)                               |
| Generation observer | Health check, finalize and the generation conditions                              | [`lmxd/src/observer.rs`](crates/lmxd/src/observer.rs)                         |
| Contract examples   | Published answers of each contract version                                        | [`contract/v1/`](contract/v1)                                                 |

Files outside `crates/` provide executable context:

| Path                                      | Purpose                                                                          |
| ----------------------------------------- | -------------------------------------------------------------------------------- |
| [`crates/lmx/tests/`](crates/lmx/tests)   | The command-line contract against a prepared guest tree, with and without `lmxd` |
| [`packaging/systemd/`](packaging/systemd) | Reference units of `lmxd` for the platform                                       |
| [`Taskfile.yml`](Taskfile.yml)            | Checks and the release build                                                     |
| [`.github/workflows/`](.github/workflows) | Pull-request checks and tag releases                                             |

## Add a fact

1. Add the value to `Status` in `lmx-model` with a doc comment, and update the examples in `contract/v1/`.
1. Add a reader module to `lmx-facts`: an I/O function and a pure parser with fixture tests.
1. Collect it in `lmx/src/status.rs` with `record`, which turns a failure into a problem instead of an error.
1. Render it in the text output and extend `crates/lmx/tests/cli.rs`.

A fact that only a guest page shows, such as `mounts` and `machine`, skips steps 1 and 3.
A new optional field is a compatible change. Renaming or removing a field needs a new contract version.
