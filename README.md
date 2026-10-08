# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

> **The owner of a LimaNix VM from inside: one command for people in the guest and one versioned contract for the host.**

`lmx` runs inside every [LimaNix](https://limanix.dev) guest.
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.
Its daemon, `lmxd`, owns the work that must not depend on a caller's session: room in the Nix store, and updates of the system from build to finalize.

[The problem](#the-problem) · [Commands](#commands) · [Host contract](docs/contract.md) · [Contributor map](ARCHITECTURE.md)

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

| Command                 | Answers or does                                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `lmx help`              | The workspace and the commands inside the VM and on the Mac; also `lmx`, `lmx -h`                                     |
| `lmx info`              | The kernel, guest disk, shared folders and failed units                                                               |
| `lmx welcome`           | The summary an interactive shell prints when it starts                                                                |
| `lmx status`            | Desired, built and booted generations; store disk usage; interfaces; failed systemd units; the state of `lmxd`        |
| `lmx doctor`            | Findings about the configuration, `lmxd` and the generations, with what to do next                                    |
| `lmx net check PORT`    | Why a port may be unreachable from the Mac: firewall rule, listener and process                                       |
| `lmx logs KIND`         | The output of the latest `lmxd` task of a kind, such as `apply`, from the journal; `--previous` reads the boot before |
| `lmx version`           | The binary version and the host contract version                                                                      |
| `lmx store reserve`     | Collects unreferenced store paths when space is low; run in `lmxd`, root only                                         |
| `lmx apply -g G`        | Builds the mounted generation G for the next boot; run in `lmxd`, root only; `--follow` streams the build             |
| `lmx apply cancel -g G` | Stops the apply of generation G; root only                                                                            |
| `lmx clipboard copy`    | Copies standard input to the Mac clipboard                                                                            |
| `lmx clipboard paste`   | Prints the Mac clipboard, if the terminal allows reads                                                                |
| `lmx session NAME`      | Opens a named session with the provider that the selected modules configure                                           |

Add `--json` to `status`, `doctor`, `net check`, `version`, `store reserve`, `apply` and `apply cancel` to answer with the [host contract](docs/contract.md).
`lmx status --short` prints only what needs attention, such as `restart` or `disk-low`, for tmux and the prompt.
`lmx status --wait converged -g G` answers once `lmxd` reports generation G booted, healthy and finalized; when the finalize fails, it answers `finalize.failed` at once, because the generation works.
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.

Started under the name `pbcopy`, `pbpaste` or `limanix-session`, the binary keeps the arguments, messages and exit statuses of the shell command it replaces.

## The daemon

`lmxd` runs as root from systemd, started at boot and through `lmx.socket`, with readiness and a watchdog.
It replaces the platform's store guard, its timer, the daily `nix-gc` timer, and the host's reserve, rebuild and prune over SSH:

- 5 minutes after boot and then every 15 minutes, it collects unreferenced store paths at idle priority when less than 20% of the store disk is free, and lists the garbage-collector roots in its log when less than 10% stays free;
- `lmx store reserve` does the same at once for the host and answers with the usage before and after;
- `lmx apply` installs the environment files of the generation the host mounted, makes room in the store and runs `nixos-rebuild boot`; a caller that disconnects leaves the apply running, and asking again attaches to it;
- after the restart into a new generation, it checks the platform units, the shared folders and a login shell of the development account, then removes the older generations and rewrites the boot entries;
- every operation is a Solti task with its output in the journal (`journalctl -u lmx`).

For an update, the host stops the system daemon and starts one from the mounted generation with `lmxd --transient`, which applies but neither guards the store nor finalizes.
A daemon never takes over a socket that another daemon still serves.

Reference units are in [`packaging/systemd/`](packaging/systemd).

## Boundaries worth knowing early

- Nothing listens on the network. `lmxd` serves a Unix socket inside the guest, and the host reaches the guest only through management SSH.
- Facts are read in the caller's process with the caller's privileges and need no daemon.
- Owner operations run only in `lmxd`; without it they fail with exit status 3 instead of running in the caller.
- The clipboard travels through the terminal with OSC 52, or through tmux inside tmux; the terminal on the Mac must allow it.
- Text is colored only on a terminal, never with `NO_COLOR` or `TERM=dumb`, with the theme of the declaration (Catppuccin Mocha by default).
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
- Release archives hold `lmx` and `lmxd`, static musl executables for `aarch64` and `x86_64` Linux.

## Development

Requirements: [Task](https://taskfile.dev/docs/installation) 3.53.1+, Git and Docker.
CI and local checks use one toolchain: tasks run Cargo in the [`ci/rust`](https://github.com/mr-chelyshkin/images) image.

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
