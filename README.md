# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

<p align="center">
  <img src=".github/assets/readme-header.png"
       alt="LimaNix lmx"
       width="100%">
</p>

`lmx` looks after a [LimaNix](https://limanix.dev) VM from the inside. In the
guest shell it answers your questions about the VM; for the Mac it speaks a
versioned JSON contract over SSH. Its daemon, `lmxd`, keeps the Nix store from
filling the disk and carries out updates from build to finalize.

[Documentation](https://limanix.dev/categories/lmx/index.html) |
[Everyday commands](guides/everyday.md) |
[Releases](https://github.com/limanix/lmx/releases)

## Get started

`lmx` comes with every LimaNix VM; there is nothing to install. New to LimaNix?
Start with
[Getting started](https://limanix.dev/categories/client/getting-started.html).
Open a guest shell on your Mac and ask it:

```console
limanix shell dev-box   # on the Mac: open a shell in the VM
lmx help                # in the guest
```

`lmx status` shows how the guest is doing, and `lmx doctor` tells you what is
wrong and what to do about it.

## Documentation

| Guide | Contents |
| -- | -- |
| [Everyday commands](guides/everyday.md) | Status, the clipboard, sessions and the welcome. |
| [How an update works](guides/updates.md) | `limanix update` from the Mac into the guest and back. |
| [Disk and the Nix store](guides/disk.md) | What `lmx` cleans up, and what it never touches. |
| [When something is wrong](guides/troubleshooting.md) | From a symptom to the command that explains it. |
| [The daemon under systemd](guides/daemon.md) | How `lmxd` runs and who may ask it what. |
| [Host contract](guides/contract.md) | The JSON the Mac and `lmx` exchange. |
| [Development](guides/development.md) | Checks, crates, rules and releases. |

The guides are published at
[limanix.dev](https://limanix.dev/categories/lmx/index.html). The
[docs repository](https://github.com/limanix/docs) assembles them with the
client and module documentation.

## Contributing

Follow the
[contribution guide](https://github.com/limanix/.github/blob/main/CONTRIBUTING.md)
when changing `lmx`. See [Development](guides/development.md) and
[Taskfile.yml](Taskfile.yml) for local checks, release builds and documentation
tasks. Use [Issues](https://github.com/limanix/lmx/issues) for questions, bug
reports and feature requests.
