# lmx contributor map

This document is the entry point for contributors and reviewers.
It explains what each crate owns, how the crates connect, and where to begin a change.

For usage, start with the [README](README.md) and the [host contract](docs/contract.md).
The [design](docs/plans/2026-10-06-guest-owner-design.md) records why the guest owner exists and what comes next.
Exact contracts live in the Rust source and its module-level documentation.

## Architecture at a glance

```text
NixOS ──► /etc/lmx/config.json ──► lmx-model::Config
                                        │
person or host ──► lmx (binary) ──► lmx-facts readers ──► statvfs, ip, systemctl, markers
                         │
                         └──► lmx-model::Envelope<T> ──► text or JSON on standard output
```

`lmx-model` holds every value that crosses a boundary: the configuration written by NixOS and the answers read by the host.
`lmx-facts` reads the running system. The `lmx` binary parses the command line, combines facts, and renders them.

## Boundaries to preserve

- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract. The `lmx version` answer still lives in the binary and moves to `lmx-model` in M1b.
- `lmx-facts` performs reads only. It never changes the system and never needs a daemon.
- A reader that runs a program splits process I/O from a pure parser; tests cover the parser with fixed output.
- The host contract changes only as described in [Change the host contract](docs/contract.md#change-the-host-contract).
- Configuration comes from NixOS. The binaries never accept configuration from the host at runtime.
- Every crate forbids unsafe Rust with `#![forbid(unsafe_code)]`.

## Source map

| Area              | Responsibility                                  | Start here                                            |
| ----------------- | ----------------------------------------------- | ----------------------------------------------------- |
| Contract types    | Configuration, envelope, error codes and status | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs) |
| Fact readers      | Disk, generations, network and failed units     | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs) |
| Command line      | Commands, output selection and exit codes       | [`lmx/src/main.rs`](crates/lmx/src/main.rs)           |
| Status            | Collecting facts and rendering them             | [`lmx/src/status.rs`](crates/lmx/src/status.rs)       |
| Contract examples | Published answers of each contract version      | [`contract/v1/`](contract/v1)                         |

Files outside `crates/` provide executable context:

| Path                                      | Purpose                                                 |
| ----------------------------------------- | ------------------------------------------------------- |
| [`crates/lmx/tests/`](crates/lmx/tests)   | The command-line contract against a prepared guest tree |
| [`Taskfile.yml`](Taskfile.yml)            | Checks and the release build                            |
| [`.github/workflows/`](.github/workflows) | Pull-request checks and tag releases                    |

## Add a fact

1. Add the value to `Status` in `lmx-model` with a doc comment, and update the examples in `contract/v1/`.
1. Add a reader module to `lmx-facts`: an I/O function and a pure parser with fixture tests.
1. Collect it in `lmx/src/status.rs` with `record`, so a failure becomes a problem instead of an error.
1. Render it in the text output and extend `crates/lmx/tests/cli.rs`.

A new optional field is a compatible change. Renaming or removing a field needs a new contract version.
