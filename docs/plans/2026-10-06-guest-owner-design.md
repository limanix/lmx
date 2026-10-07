# LimaNix guest owner: `lmx` and `lmxd`

Status: design agreed on 2026-10-06. M1a is implemented: repository, contract model, facts,
`lmx status`, release pipeline. See [the M1a plan](2026-10-06-m1a-lmx-foundation.md). M1b is
implemented: help, info, welcome, clipboard and sessions in the binary. See
[the M1b plan](2026-10-06-m1b-lmx-user-surface.md). The S1 spike passed; its findings
for M2 are in [the S1 results](2026-10-07-s1-solti-spike.md). M2 is implemented: `lmxd` keeps room
in the Nix store and answers `lmx store reserve`. See [the M2 design](2026-10-07-m2-lmxd-store-design.md)
and [the M2 plan](2026-10-07-m2-lmxd-store.md).

Scope: this repository, plus the changes it requires in `client` (host and
platform base) and `modules` (catalog).

## 1. Problem

LimaNix VMs are long-lived development environments, but nothing inside the
guest owns them. Today:

- **Host reaches in with literal commands.** It sends about ten of them over
  management SSH (`client/internal/guest`):
  - installs the ENV files;
  - runs a transient `systemd-run` rebuild with a hand-written cancellation
    race;
  - probes the disk with `stat` and the address with `ip`;
  - collects garbage and prunes generations;
  - runs `true` after a restart.
- **Disk maintenance lives in four places:**
  - the `limanix-store-guard` timer (bash);
  - `nix-gc.timer` (daily, no options);
  - `min-free`/`max-free` during builds;
  - `Reserve` and `Prune` called from the host.
- **The host does not know which NixOS generation is booted.** It records only
  the ID of the inputs it prepared. `ready` in `limanix list` is a saved
  result, not an observation.
- **The guest interface is shell scripts.** `lmx` (help, info, welcome), a
  223-line `welcome.sh`, `pbcopy`, `pbpaste` and `limanix-session` take their
  values from wrappers they are baked into.

Every new guest concern adds a mechanism in another place. The inode work alone
added three.

## 2. Goals and non-goals

Goals:

- One owner of the VM from inside. It covers in-guest apply steps, store and
  disk maintenance, the real guest state, diagnostics, and the user-facing
  guest surface.
- One typed, versioned host–guest contract.
- Guest state derived from reality instead of remembered.
- The base shell scripts and the host's literal guest commands replaced.
- A runtime built on Solti (Taskvisor and the Solti SDK).

Non-goals:

- No network listeners. The host keeps using management SSH.
- No supervision of services declared by catalog modules. They stay with
  systemd.
- No firewall or port-forward configuration at runtime.
- No control plane, discovery or podium.
- No durable job queue.

## 3. Responsibility zones

| Zone | Owns | Does not |
|---|---|---|
| Declaration: `limanix.toml` and the catalog | What is installed and how it is configured; policies: disk thresholds, theme, maintenance | — |
| Host: `limanix` | Turning the declaration into generation inputs; the VM as a machine: Lima, resources, disk size, network, mounts, power; managed homes; operation records and locks; orchestration of operations that need a VM restart; presentation on the Mac | Run guest logic as literal commands; decide when to collect the store |
| Lima | Virtualization, cidata, SSH transport, host agent, guest agent | Unchanged |
| Guest owner: `lmx` and `lmxd` | In-guest apply steps; store and disk maintenance; real guest state; diagnostics; the user-facing guest surface: runtime theme, own UI, clipboard, sessions; later user jobs | Touch power, resources, Lima or Mac files; supervise module services (it only observes them); configure the firewall or forwards |
| NixOS and systemd | Boot, activation, units, users, sudo, module services, running `lmxd`; the safety floor that works without `lmxd` (`min-free`/`max-free`) | — |

Invariants:

1. The host acts in the guest only through `lmx <operation>` over SSH. The
   interactive shell is the only exception. It is transport for a person.
2. The guest never makes VM-level decisions. It reports conditions and the host
   decides.
3. Policies come only from the declaration. `lmx` reads its configuration from
   the built system, never from the host at runtime.
4. The host is the source of truth for what the user asked for: operations and
   their records. The guest is the source of truth for what the VM actually is.
   `limanix list` shows both.
5. If `lmxd` is down, the guest still works. The Nix safety floor stays, `lmx`
   facts and `lmx doctor` still work, and the host reports the owner as
   unavailable.

## 4. Components

One repository (`limanix/lmx`), one Cargo workspace, one release, two binaries:

| Binary | Runs as | Lifetime | Domains | Solti |
|---|---|---|---|---|
| `lmx` | The caller: the dev user, or root through `sudo` from the host | One command | Facts, diagnostics, user surface, client of `lmxd`, the host contract | No |
| `lmxd` | Root, from systemd | From boot | System lifecycle (apply, finalize, generations), store and disk, conditions | Taskvisor, `solti-core`, `solti-exec`, `solti-api` |
| User-jobs daemon (later) | Dev user, from the systemd user manager (linger is on) | From boot | User workloads with cgroup limits | `solti-core`, `solti-exec` |

Why this split:

- **A binary boundary is a privilege and lifetime boundary.** `welcome` runs
  when an interactive shell starts, so `lmx` stays light and never links the
  daemon runtime. Root code never runs in the user's terminal.
- **System and store share one process.** They have the same privilege and
  lifetime and must coordinate: no collection during a build.
- **User workloads get their own process.** They run with other privileges and
  must not take the system owner down with them.

`lmx` is a multi-call binary. `pbcopy`, `pbpaste` and `limanix-session` are
names for it, dispatched on `argv[0]`, so the public guest contract keeps its
commands.

Crates follow domains. M1 creates `lmx-model` and `lmx-facts`, because `lmxd`
consumes them in M2. The other crates are added when needed:

- `lmx-model`: contract types, error codes, configuration schema.
- `lmx-facts`: pure readers of the system.
- `lmx-state`: derivation of desired, built and booted generations and their
  conditions. It is pure; `lmxd` uses it, and so does `lmx` when the daemon is
  down.
- `lmx-ipc` (M2): gRPC over a Unix socket, server and client.
- Binaries `lmx` and `lmxd`. UI, theme, clipboard and sessions start as modules
  of `lmx`.

## 5. Command kinds

| Kind | Runs in | Source of truth | Without `lmxd` | Examples |
|---|---|---|---|---|
| Facts | The caller's process | The system: statfs, system profile, `/run/booted-system`, systemd (`systemctl` in M1a), interfaces (`ip -j` in M1a) | Work | Disk and inodes, generations, failed units, interfaces, listening ports |
| Owner operations | `lmxd` only | Solti tasks: status, history, output | Fail with `owner.unavailable` | Apply, cancel, store collect and reserve, maintenance, logs |
| Caller context | The caller's process, with its TTY and environment | The terminal and environment | Work | `pbcopy`, `pbpaste`, sessions, welcome, theme in the current shell |

- **Composite commands combine kinds.** `lmx status` is facts plus owner state
  when `lmxd` is reachable. `lmx doctor` must work without `lmxd`, because one
  of its jobs is diagnosing it.
- **One executor.** Owner operations have exactly one executor, `lmxd`. There
  is no silent in-process fallback, for two reasons:
  - it would race a daemon that is restarting;
  - an operation running inside an SSH session dies with the session.

  The first create starts a transient `lmxd` instead (section 7).
- **Facts run with the caller's privileges.** The host always calls through
  `sudo` and sees everything. A user may see a partial view, such as other
  users' GC roots or processes behind other users' ports. The output marks
  this.

## 6. Update model

The host starts the build explicitly. Everything inside the guest belongs to
the guest. The state is derived from reality, not stored.

| Generation | Source |
|---|---|
| desired | The generation mounted at `/mnt/limanix`. The host commits desired state by mounting it. |
| built | The current system profile |
| booted | `/run/booted-system` |

The generation ID is added to `runtime.json` and baked into the system
(`/etc/lmx/config.json`), so all three can be compared:

| desired / built / booted | Condition |
|---|---|
| G / F / F | `OutOfDate`: apply needed |
| G / G / F | `RestartRequired` |
| G / G / G, healthy, no older generations | `Converged` |
| G / G / G, health check failed | `Degraded`; the previous generation is kept |

```
host                                    guest
lmx store reserve                  →    collect if needed (warning only)
stop → Lima edit (/mnt/limanix=G) → start
lmx apply -g G --follow            →    ENV → reserve → nixos-rebuild boot
                                   ←    output … RestartRequired
restart VM
lmx status --wait converged -g G   →    on boot: booted == G → health check → finalize
                                   ←    Converged | Degraded
record ready / error
```

Properties:

- **No rebuild on start.** `limanix start` after a failed update rebuilds
  nothing, and `limanix list` shows `OutOfDate`.
- **Finalize after boot runs on its own and is safe.** It runs only when
  desired, built and booted agree and the health check passes.
  - The health check replaces the host's `true`: the dev user can run a
    command, and the platform units are alive (`lmxd`, `sshd`, mounts).
  - Failed module units become their own condition and do not block finalize.
- **Updates resume.** If the host dies mid-build, the guest finishes and
  reports `RestartRequired`. The next update skips the build.
- **Cancellation is explicit.** It is `lmx apply cancel -g G`. The task has a
  name before its process starts, so the launch race in `rebuild.go` goes away.
  An SSH drop only detaches the observer.
- **Rollback becomes possible.** The previous generation is kept until
  `Converged`, which allows a later `limanix rollback`.

## 7. Apply executor

Apply always runs the `lmxd` pinned by the mounted generation, which is the
version the client brought:

1. The host stops the system `lmx.socket` and `lmx.service`, if present.
2. It builds the pinned package from the mounted flake:
   `sudo nix build --no-link --print-out-paths path:/mnt/limanix/flake#lmx`.
3. It starts a transient daemon from that package:
   `sudo systemd-run --unit=lmxd-apply-<G> <package>/bin/lmxd --transient`.
4. It runs `lmx apply` from the same package against that daemon.
5. After the restart, the new system `lmxd` finalizes.

Why:

- **One path for create and update.** The first create has no system daemon
  either.
- **Fixes land immediately.** A fix to apply takes effect on the update that
  ships it.
- **The critical operation always runs the client's version.**
- **The cost is small.** Maintenance pauses during apply, but the VM restarts
  anyway.

Version compatibility is then needed only for `status` and `store reserve`. If
the VM's `lmx` is too old, the host reports that the VM needs an update and
skips `reserve`, which only warns.

The transient daemon binds the socket itself. It takes its configuration from
the mounted inputs of the desired generation (G, user, thresholds) and runs
only apply and status, without maintenance.

## 8. Host contract

Transport: `sudo lmx <command> --json` over management SSH. There is no other
channel.

Every command uses the same envelope:

```json
{"contract": 1, "ok": true,  "data": {}}
{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {}}}
```

Error codes are stable strings: `owner.unavailable`, `apply.build_failed`,
`apply.cancelled`, `disk.low`, `network.unreachable`, `permission.denied`,
`generation.mismatch`, `contract.unsupported`. The message is for people, the
code is for the host.

Exit codes:

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Operation failed; details in the JSON |
| `2` | Usage error |
| `3` | Owner unavailable |
| `130` | Cancelled |

Long operations with `--follow` write JSON Lines: output lines, phase changes,
and a final `result` event. The host prints the output and decodes the result.
An SSH drop does not cancel the operation.

| Command | Host calls it | Returns |
|---|---|---|
| `lmx status` | `list`; before and after operations | Generations, conditions, disk, interfaces, failed units, owner state. Replaces the separate disk and address probes. |
| `lmx store reserve` | Before stopping a running VM | Usage before and after, freed bytes, `DiskLow`. Warning only. |
| `lmx apply -g G --follow` | Create and update, after Lima edit and start | Output, then `RestartRequired` or an error. Idempotent: if G is already built it returns `RestartRequired`; it attaches to a build of G in progress; mismatched inputs return `generation.mismatch`. |
| `lmx apply cancel -g G` | On Ctrl-C | The cancellation result |
| `lmx status --wait converged -g G` | After the restart | `Converged`, `Degraded` with a reason, or a timeout |
| `lmx doctor`, `lmx net check PORT` | New `limanix doctor` and `limanix net check` | Facts and findings |

The contract lives in `lmx-model`, with golden examples for each contract
version. A JSON Schema is deferred; the golden examples define v1 until then. The client pins the `lmx` release and tests its
decoders against those examples.

## 9. Inside `lmxd`

### Configuration

Nix renders `/etc/lmx/config.json` from two sources:

- the platform: VM name, architecture, user, generation ID, disk thresholds,
  platform units for the health check, session provider;
- the catalog: theme, and later maintenance declared by modules.

`lmxd` and `lmx` read the booted generation's file. It replaces the values
baked into the `lmx` wrapper, `/etc/limanix/workspace`, and the thresholds
inside `store-guard.sh`.

### Tasks

Tasks are custom workload kinds under `lmx.limanix.dev/v1`, not Embedded, so
they show up the same way in `lmx ps`, logs and history.

| Task | Does | Slot | Started |
|---|---|---|---|
| `SystemApply{G}` | Check the mounted G → install ENV → reserve through `StoreCollect` → `nixos-rebuild boot` | `system`, replace | Only by the host |
| `SystemFinalize{G}` | Health check → delete older generations → `switch-to-configuration boot` → collect | `system` | Automatically when desired = built = booted and there is something to finalize |
| `StoreCollect` | `nix-store --gc` | `store`, queue | By reserve, guard and apply |
| `StoreGuard` | Every 15 minutes, compare usage with the thresholds and start `StoreCollect` if needed; below the minimum, report GC roots | Periodic | Always; skips a tick while `system` is busy |

### Runtime

- **Observer.** A periodic task, plus a recomputation after each operation. It
  derives conditions from facts and stores nothing.
- **Execution.** Every external command goes through `solti-exec`
  subprocesses: explicit `PATH`, cleared environment, process-group cleanup,
  cancellation, output capture.
- **History.**
  - `solti-core` keeps runs in memory, and apply always ends with a reboot.
    So task output and status changes are also written to journald with
    `LMX_TASK` and `LMX_GENERATION` fields.
  - `lmx logs apply --previous` reads the journal.
  - The platform sets an explicit journald size limit.
- **Transport.** gRPC over `/run/lmx/lmx.sock`, with two services:
  - the `solti-api` Task service for the operation lifecycle: create, watch,
    output, cancel, history;
  - an `lmx` service for aggregated status and synchronous reserve.

  The generated Task client already exists in `solti-api`.
- **Authorization.** `SO_PEERCRED`, read from tonic's Unix connection info and
  checked by a tower layer in front of the Solti services.
  - Root may do everything.
  - Other users may only read: get, list, watch, output.

  The `solti-api` authenticator only sees bearer credentials, so this check
  lives in `lmxd`.
- **Lifecycle.**
  - `lmx.socket` and `lmx.service` use socket activation, so the socket exists
    while the daemon restarts.
  - `Type=notify`, with a watchdog fed by a periodic task.
  - The daemon starts at boot, because maintenance and finalize must run.
  - SIGTERM triggers a graceful Taskvisor shutdown: tasks cancel cooperatively
    and subprocess groups are killed.

## 10. User surface and network

### Theme

Today the palette exists as data (`_shared/palette.toml`), but it is wired in
several separate ways:

- each module that reads it picks `mocha` itself;
- astronvim, harlequin and posting set the theme by name;
- `welcome.sh` repeats the colors as RGB.

The theme will be defined once, in the declaration: a `_shared/theme.nix`
capability area in the catalog (flavor and palette). Every module and `lmx`
consume it. `lmx` renders its own UI from that palette.

Runtime switching comes later: `lmx theme set`, or following the Mac's light
and dark appearance. Nix builds configurations for every flavor, and `lmx`
switches the current one and signals the tools that can reload.

### Clipboard and sessions

The implementations move into `lmx` as `lmx clipboard copy|paste` and
`lmx session NAME`. `pbcopy`, `pbpaste` and `limanix-session` stay as names.
These are caller-context commands, because OSC 52 must reach the caller's
`/dev/tty`.

### Network

The guest owner only observes and diagnoses the network:

- ports belong to the declaration (`network.ports`);
- address, network and forwards belong to the host and Lima.

`lmx net check PORT` automates the guest side of the "Check a connection in
order" guide: listener, bound address, process, firewall rule and protocol.
`limanix net check` adds the VM state, the address, and a TCP attempt from the
Mac. When a rebuild fails, `lmxd` also checks whether the binary cache is
reachable.

## 11. Delivery

- **Releases.** `limanix/lmx` publishes static musl binaries for `aarch64` and
  `x86_64` Linux as `lmx-<version>-<system>.tar.gz`, where `<system>` is the
  Nix system name. Each archive comes with a SHA-256 checksum file.
- **Pin.** The platform base in the client pins the release version and one
  SHA-256 per system in a JSON file next to its Nix modules. Updating `lmx`
  means updating that file.
- **Guest build.** NixOS fetches the pinned archive with `fetchurl` and
  installs it as its own small store path. This is the same path as all other
  guest software. It needs no build-time download or embedding in the client.
  It adds no network requirement either, because rebuilds already fetch
  Nixpkgs from GitHub.
- **Flake output.** The generated flake also exposes the package as
  `packages.<system>.lmx`. The transient daemon of section 7 is built from it
  before the system itself.
- **Platform base.** It adds the multi-call names, `/etc/lmx/config.json`,
  the units and the welcome hook.
- **Existing VMs.** They get `lmx` with their next `limanix update`. Until
  then, the host falls back to its current probes when `lmx` is missing.

## 12. Milestones

| Step | Delivers | Removes |
|---|---|---|
| M1 | `lmx` CLI: multi-call; help, info, welcome; clipboard; session; `status --json` facts; contract v1. Release pipeline; pinned release fetched by the platform; `/etc/lmx/config.json`; generation ID; `limanix list` through `lmx status`, with a fallback. Split into M1a (repository, model, facts, `status`, release), M1b (user surface) and M1c (client and platform) | `lmx.sh`, `help.sh`, `info.sh`, `welcome.sh`, `pbcopy.sh`, `pbpaste.sh`, `session.sh`, the baked wrapper |
| S1 (parallel to M1) | Throwaway spike that checks three things: the Solti set (core, Taskvisor, exec, tonic) builds as static musl for both architectures; gRPC works over a Unix socket with `peer_cred`; socket activation and the watchdog work in a LimaNix guest | — |
| M2 | `lmxd` with the store domain (guard, collect, reserve), conditions and IPC; `lmx` as its client; host reserve through `lmx store reserve` | `store-guard.sh` and its timer, `nix-gc.timer`, host `Reserve` |
| M3 | `SystemApply` and `SystemFinalize`; transient daemon from the mount; new host create and update flow; resume; cancel | Supervision in `rebuild.go`, `prune.go`, `installEnvironment`, the host `true` check |
| M4 | `doctor`, `net check`, theme capability in the catalog, journald history and limit, `status --short` for tmux and the prompt | Per-module theme wiring |
| Later | User-jobs daemon; projects domain; maintenance declared by modules | — |

Why this order:

- **M1 first.** Every later step needs the delivery pipeline, and it is best
  proven on the simplest payload, without root and without touching update.
- **S1 alongside M1.** It removes the main architectural risk before M2.
- **M2 before M3.** This goes from safe to dangerous. Apply changes the host's
  most critical flow, so it comes after the daemon, IPC and delivery are
  proven.

## 13. Solti

Used: Taskvisor, `solti-core`, `solti-exec`, `solti-api` (gRPC) and
`solti-observe` (journald).

Not used: discovery, TLS, Prometheus, containerd, cron, chains, podium.

Current state:

- **Uncommitted changes are not needed by `lmx`.** These are in the SDK: the
  new `solti-cron`, the chain cancellation fix, the `google.rpc.Status`
  envelope for gRPC errors, and the discovery request timeout. Podium's
  `remote-proto` branch is not needed either.
- **No blockers found.** `solti-api` and `solti-core` make no TCP assumptions,
  and the gRPC Task client is exported.

Desired improvements:

- Let `solti-api` authentication see connection identity. Any local agent
  would benefit.
- Possibly move a journald sink for task output and state into `solti-observe`,
  if it proves general.

S1 still has to confirm static musl builds of the selected crates. `lmx` pins
an exact Solti version.

## 14. Open questions

- **Projects as a domain.** It would cover mounts, per-project sessions
  (`tmux-project` in cozy), nix-direnv roots, devshell prewarm after update,
  and jobs per project.
- **Maintenance declared by catalog modules.** Docker has no prune today. The
  option shape and the ownership rules are open.
- **Health check.** The exact policy and the list of platform units.
- **Compatibility window** for `status` across `lmx` versions.
- **journald size limit:** the value.

## 15. Findings outside this design

- `/etc/limanix/environment*` is installed with mode `0644`, so any guest
  account can read ENV values. An interim fix to `root:<user> 0640` is filed
  separately.
- There is no journald size limit.
- Nothing maintains Docker data.
- `nix-gc.timer` duplicates the store guard.
