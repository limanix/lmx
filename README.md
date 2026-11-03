[![Release](https://img.shields.io/github/v/release/limanix/lmx)](https://github.com/limanix/lmx/releases)
[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

<div align="center">
<img src=".github/assets/readme-header.png" alt="LimaNix" width="100%">

<p align="center">
<a href="https://docs.limanix.dev/quick-start"><code>→ Get started</code></a>
<a href="https://docs.limanix.dev/"><code>Documentation</code></a>
<a href="https://docs.limanix.dev/contributing/"><code>Contributing</code></a>
</p>

<p align="center"><sub>macOS 26 or newer | Apple Silicon and Intel</sub></p>
</div>

## Guest tools and daemon

`lmx` and `lmxd`, the guest command and daemon for
[LimaNix](https://limanix.dev). Inspect and maintain your NixOS development VM
from the inside. Both are included in every LimaNix VM.

- **Explore your environment.** View VM status with `lmx status` and find
  commands provided by enabled modules with `lmx help`.
- **Diagnose problems.** Check guest health and network ports, and inspect
  daemon task logs.
- **Manage system updates.** `lmxd` builds system generations for the next boot,
  checks their health after restart and reclaims unused Nix store paths.
- **Work across Mac and VM.** Use the Mac clipboard through your terminal, link
  configured dotfiles and open named sessions through a configured provider.

Read the [lmx guide](https://docs.limanix.dev/inside/lmx) and
[command reference](https://docs.limanix.dev/reference/lmx). Release archives
are available on [GitHub Releases](https://github.com/limanix/lmx/releases).

## Development

> Requires [Task](https://taskfile.dev) 3.53.1+ and Docker.

Run Rust tests with `task ci/rust/test`; use `task -l` to list checks and build
commands.

Check Markdown with `task ci/markdown/fmt`; format it with `task markdown/fix`.

## Contributing

See the [contribution guide](https://docs.limanix.dev/contributing/) for issues,
pull requests and releases, and the [host contract](contract/README.md) for the
JSON interface between the client and guest.
