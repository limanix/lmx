[![Release](https://img.shields.io/github/v/release/limanix/lmx)](https://github.com/limanix/lmx/releases)
[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

<p align="center">
  <img src=".github/assets/readme-header.png"
       alt="LimaNix lmx"
       width="100%">
</p>

<p align="center">
  <a href="https://docs.limanix.dev"><b>Docs</b></a>
  &nbsp;·&nbsp;
  <a href="https://docs.limanix.dev/inside/lmx"><b>Using lmx</b></a>
  &nbsp;·&nbsp;
  <a href="https://docs.limanix.dev/reference/lmx"><b>Command reference</b></a>
  &nbsp;·&nbsp;
  <a href="https://docs.limanix.dev/architecture"><b>How it works</b></a>
  &nbsp;·&nbsp;
  <a href="https://github.com/limanix/lmx/releases"><b>Releases</b></a>
</p>

The guest side of [LimaNix](https://limanix.dev), which runs NixOS development
VMs on a Mac. [`limanix`](https://github.com/limanix/client) creates and updates
a VM from one file, the [catalog](https://github.com/limanix/modules) gives it
shells, toolchains and services, and `lmx` runs it from the inside. Every VM
comes with `lmx`; there is nothing to install.

```text
     Mac                       ┌─ VM (NixOS) ───────────────────────────────┐
┌───────────┐   SSH, --json    │ ┌───────────┐  local socket  ┌───────────┐ │
│  limanix  │ ─────────────────┼>│    lmx    │ ─────────────> │   lmxd    │ │
└───────────┘                  │ └───────────┘                └───────────┘ │
                               │       ^                                    │
                               │  your shell                                │
                               └────────────────────────────────────────────┘
```

| Part | What it does |
| -- | -- |
| `lmx` | The command in the VM, for you and for `limanix`: the welcome, `lmx status`, `lmx doctor`, `lmx help`, dotfile links and the Mac clipboard |
| `lmxd` | A root daemon: builds each new system, checks it after the restart, removes the old one and keeps the Nix store from filling the disk |
| [Host contract](contract/README.md) | The versioned JSON that `limanix` reads from `lmx`, with examples in `contract/v1` |

## Development

Every check runs in the container that CI uses; you need
[Task](https://taskfile.dev) 3.53.1 or newer and Docker. `task --list` shows the
checks, and `task release/build ARCH=aarch64` builds a release archive in
`dist/`. A tag `v<version>` on `main` publishes the archives for `aarch64` and
`x86_64`; the client then pins the release in its `lmx.json`.

## Contributing

Follow the
[contribution guide](https://github.com/limanix/.github/blob/main/CONTRIBUTING.md).
Use [Issues](https://github.com/limanix/lmx/issues) for questions, bug reports
and feature requests. Licensed under [Apache 2.0](LICENSE).
