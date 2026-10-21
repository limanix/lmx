# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)
[![Release](https://img.shields.io/github/v/release/limanix/lmx)](https://github.com/limanix/lmx/releases)

<p align="center">
  <img src=".github/assets/readme-header.png"
       alt="LimaNix lmx"
       width="100%">
</p>

**Guest CLI and daemon for [LimaNix](https://limanix.dev) development
environments.**

LimaNix runs NixOS development VMs on macOS. `lmx` is the part that lives
inside every VM: a command that tells you what is going on in the guest, and a
daemon, `lmxd`, that keeps the VM healthy and carries out the updates you start
on your Mac. It comes with every LimaNix VM; there is nothing to install.

[Documentation](https://limanix.dev/categories/lmx/index.html) |
[Getting started](https://limanix.dev/categories/client/getting-started.html) |
[Releases](https://github.com/limanix/lmx/releases)

## Features

- **Know what is going on.** `lmx status` shows the VM at a glance, and
  `lmx doctor` names a problem and what to do about it. Every new shell opens
  with a short welcome.
- **Help for your tools.** `lmx help go` shows what a module installed: its
  commands, common tasks and guide.
- **Your Mac's clipboard.** `pbcopy` and `pbpaste` reach the Mac clipboard,
  also from inside tmux.
- **Your dotfiles in every VM.** One directory on the Mac, linked into the home
  of each VM; edits are live.
- **Sessions that survive.** `limanix shell --session work` returns you to the
  same terminal session.
- **Safe updates.** `lmxd` builds the new NixOS generation, checks it after the
  restart, and only then removes the old one.
- **A disk that does not fill up.** `lmxd` cleans the Nix store before it runs
  out of space.

## Where it fits

| Repository | Runs on | Role |
| -- | -- | -- |
| [client](https://github.com/limanix/client) | Mac | `limanix` creates, updates and opens the VMs |
| [modules](https://github.com/limanix/modules) | VM | The NixOS catalog: shells, toolchains and services |
| **lmx** | VM | The guest command and daemon; answers the Mac over a versioned contract |

## A quick look

```console
limanix shell dev-box   # on the Mac: open a shell in the VM
lmx help                # the VM, its modules and commands
lmx status              # generations, disk and network
lmx doctor              # what is wrong, and what to do
```

## Contributing

Follow the
[contribution guide](https://github.com/limanix/.github/blob/main/CONTRIBUTING.md)
and [Development](guides/development.md). Use
[Issues](https://github.com/limanix/lmx/issues) for questions, bug reports and
feature requests.

Licensed under [Apache 2.0](LICENSE).
