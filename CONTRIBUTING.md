# Contributing to lmx

This repository holds `lmx`, the command in every LimaNix VM, and `lmxd`, its
root daemon. The
[organization guide](https://github.com/limanix/.github/blob/main/CONTRIBUTING.md)
covers issues, release labels and the license; this guide covers the work in
this repository.

## Set up

You need [Task](https://taskfile.dev) 3.53.1 or newer and Docker. Every check
runs in the container that CI uses; `task --list` shows them.

## Find your way

| Crate | Holds |
| -- | -- |
| `lmx` | The command: the welcome, `lmx status`, `lmx doctor`, `lmx help`, dotfile links and the clipboard |
| `lmxd` | The daemon: system builds, checks after a restart and keeping the Nix store from filling the disk |
| `lmx-model` | Data shared by `lmx`, `lmxd` and the host: the configuration the platform writes, its theme included, and the host contract |
| `lmx-facts` | Readers of the running VM that the commands report from |
| `lmx-ipc` | Calls between `lmx` and `lmxd` |

The [host contract](contract/README.md) is the versioned JSON that `limanix`
reads from `lmx --json`, with examples in `contract/v1`. A change to it ships
with the client support that reads it.

## Before you open a pull request

| Step | Command |
| -- | -- |
| Format and lint | `task ci/rust/fmt`, `task ci/rust/lint`; `task rust/fmt/fix` rewrites files |
| Test | `task ci/rust/test` |
| Check documentation and dependencies | `task ci/rust/docs`, `task ci/rust/audit` |
| Check Markdown | `task ci/markdown/fmt`; `task markdown/fix` rewrites files |
| Update the command reference after a command changes | `task reference/fix` |

`reference.json` is the source of the `lmx` pages on docs.limanix.dev; a test
fails when it is out of date. Colors come from the theme the platform writes
into the configuration: draw with the roles of its palette, as
`crates/lmx/src/palette.rs` does, never with a theme's name.

## Releases and the client

A tag `v<version>` on `main` publishes the archives for `aarch64` and `x86_64`.
The client pins a release in `internal/nixos/resources/base/lmx.json`, and its
platform modules write the configuration that `lmx-model` reads. A change to
that configuration, such as a new field, therefore needs an `lmx` release first;
the client pull request then moves the pin and writes the new field together.
