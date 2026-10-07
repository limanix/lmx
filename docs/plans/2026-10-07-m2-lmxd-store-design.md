# M2: `lmxd` and the store domain

Status: design agreed on 2026-10-07 and implemented; see [the M2 plan](2026-10-07-m2-lmxd-store.md).
It refines M2 of [the guest owner design](2026-10-06-guest-owner-design.md) (section 12) with the
findings of [S1](2026-10-07-s1-solti-spike.md).

## Scope

In M2:

- `lmxd`, the root daemon on Solti, with the store domain: guard, collect and reserve;
- owner state: conditions and running operations;
- IPC between `lmx` and `lmxd`;
- `lmx store reserve` for the host, and owner state in `lmx status`;
- contract additions, reference systemd units, and `lmxd` in the release archive.

Not in M2:

- generation conditions, apply and finalize: M3;
- `lmx ps`, `lmx logs`, journald history and field names: M4;
- the client and the platform base: M1c. It renders `/etc/lmx/config.json` and the units,
  calls `lmx store reserve`, and removes `store-guard.sh`, its timer, `nix-gc.timer` and the
  host `Reserve`.

## Behaviour to keep

M2 reproduces three things that M1c removes.

**`limanix-store-guard`** (`store-guard.sh` and `platform.nix` in the client's platform base):

- A oneshot unit, run as root with `Nice=19` and `IOSchedulingClass=idle`. Its timer fires
  5 minutes after boot, then 15 minutes after each activation.
- It reads `stat -f /nix/store`: total and free blocks (`f_bfree`, which includes the root
  reserve), and total and free inodes.
- "Below p%" means `free × 100 < total × p`, for blocks or for inodes. A total of zero is
  skipped, as on file systems without an inode table.
- Below `collect_percent` (20) it prints
  `Less than <collect_percent>% of the guest disk is free; collecting unreferenced store paths.`
  and runs `nix-store --gc --quiet`. If the collection fails, it exits 1.
- Still below `minimum_percent` (10), it prints
  `Less than <minimum_percent>% of the guest disk is still free. Other garbage-collector roots:`
  and the output of `nix-store --gc --print-roots`. Lines that match `^"?/proc/`, `^"?/run/`,
  `^"?/nix/var/nix/profiles/system` or `{censored}` are left out.
- If usage cannot be read, it prints `Store file-system usage cannot be read.` on standard
  error and exits 1.

**`nix-gc.timer`** comes from `nix.gc.automatic`: a daily `nix-collect-garbage` at 03:15. It
duplicates the guard. After M1c removes it, unreferenced paths are collected only when less
than 20% is free.

**The host `Reserve`** (`client/internal/guest/disk.go`):

- It is called in two places: before an update stops a running VM, and in `Apply` before the
  rebuild.
- It takes the same measurement over SSH. Below 20% it runs `sudo nix-store --gc --quiet`.
- If the disk is still below 10%, it returns `DiskError`. `Update` prints this as a warning;
  `Apply` ignores it.
- A failed probe is skipped silently. A failed collection matters only when the disk stays
  low.

`min-free` and `max-free` in `nix.conf` stay. They are Nix's own floor during builds.

## Components

| Crate | Change |
|---|---|
| `lmx-model` | `DiskUsage::below(percent)`; reserve result and details; `Status.owner`; `ErrorCode::DiskUnreadable`; `Config.tools` gains `nix_store`, `nice`, `ionice` and `grep` |
| `lmx-ipc` (new) | `lmx.v1.Owner` proto and its tonic code, the socket path, and a Unix socket client. It has no Solti, so `lmx` stays free of it. |
| `lmxd` (new) | Configuration, Solti supervisor and runners, store service, owner service, Task API, peer authorization, systemd integration, logging |
| `lmx` | `lmx store reserve`; owner state in `lmx status` |

Tools run by absolute path from `Config.tools`. Tasks get a cleared environment, and the NixOS
shell has no default `PATH` (S1, finding 8). `/bin/sh` is used as is.

### Tasks

`lmxd` runs two custom workload kinds under `lmx.limanix.dev/v1`.

- One runner declares both kinds. It turns each task into a `solti.io/v1` subprocess task, and
  a private `SubprocessRunner` builds that task.
- The private router keeps raw subprocess workloads out of the public API.
- Output goes to the task's stream. A `TaskOutputSink` also sends it to the journal.

| Kind | Name | Runs | Slot | Timeout |
|---|---|---|---|---|
| `StoreCollect` | `store-collect-<n>` | `nix-store --gc --quiet`; with `priority: idle`, `nice -n 19 ionice -c 3 nix-store --gc --quiet` | `store`, queued | 2 hours |
| `StoreRoots` | `store-roots-<n>` | `nix-store --gc --print-roots`, filtered through `grep -E -v` as in the guard, under `/bin/sh`; tool paths are passed as arguments; the exit status is ignored | `store`, queued | 10 minutes |

- **Restarts.** Neither kind restarts.
- **Names.** `<n>` counts from 1 for each daemon start. State is kept in memory, so names never
  collide.
- **One collection at a time.** At most one `StoreCollect` is active. Under one lock, `lmxd`
  finds the active one or creates it. Every caller waits for that one.

## Store behaviour

**Measurement.** `statvfs("/nix/store")` through `lmx-facts`, with free bytes from `f_bfree`.
`DiskUsage::below` follows the guard and the host exactly.

**Guard.** A loop inside `lmxd`.

- **Timing.** The first tick comes 5 minutes after boot, counted from the system uptime. A
  daemon started later than that ticks at once. Each later tick comes 15 minutes after the
  previous one ends.
- **One tick:**
  1. Measure. If usage cannot be read, log `Store file-system usage cannot be read.` as an
     error and end the tick.
  2. If usage is not below `collect_percent`, do nothing.
  3. Otherwise, log the guard's first message. Run `StoreCollect` with `priority: idle`, or
     wait for the active one.
  4. If the collection fails, log an error and end the tick. There is no second measurement
     and no roots.
  5. Measure again. If usage is below `minimum_percent`, log the guard's second message and
     run `StoreRoots`.
- **Later.** M3 skips the tick while the `system` slot is busy.

**Reserve.** `Owner.Reserve`, step by step:

1. Measure `before`. If usage cannot be read, the error is `disk.unreadable`.
2. If usage is not below `collect_percent`, succeed with
   `{before, after: before, freed_bytes: 0, collected: false}`.
3. Otherwise, run `StoreCollect` with `priority: normal`, or wait for the active one. Then
   measure `after`.
4. `freed_bytes = after.free_bytes − before.free_bytes`, saturating at zero.
5. If `after` is below `minimum_percent`, the error is `disk.low`. Its details are
   `{before, after, freed_bytes}`, plus `collect_error` when the collection failed. `StoreRoots`
   runs for the journal.
6. Otherwise, succeed with `collected: true`. If the collection failed, `collected` is `false`
   and the journal gets a warning.

The collection belongs to `lmxd`. If a client disconnects, for example after Ctrl-C on the
host, the collection goes on.

**Owner state.** `Owner.Status` returns:

- `version`: the version of `lmxd`.
- `conditions`: computed from a fresh measurement and never stored. M2 has one condition,
  `DiskLow`, set when usage is below `minimum_percent`.
- `operations`: the active `lmx.limanix.dev` tasks, each with kind, name, phase and start time.

## IPC and security

- **Transport.** gRPC (tonic 0.14) on `/run/lmx/lmx.sock`, with mode `0666`. `SO_PEERCRED`
  decides what each caller may do.
- **Identity.** On every request, an interceptor puts the caller's uid, gid and pid into an
  `ApiIdentity` (S1).
- **Privileged callers.** These are uid 0 and the user `lmxd` runs as.
  - In production both are root.
  - In tests, the daemon's own user can call `Reserve` without sudo. No privilege is gained.
- **`lmx.v1.Owner`.**
  - `Status` is open to everyone. `Reserve` is for privileged callers only, and the handler
    checks this.
  - Domain failures travel in the response as `oneof {result, error{code, message, details}}`,
    with contract codes.
  - A gRPC status means a transport failure.
- **Solti Task API.**
  - Everyone can Get, List, Watch, ListRuns and StreamLogs.
  - Privileged callers can Cancel and Delete.
  - Nobody can Create or Apply, because only `lmxd` creates tasks.
  - No `lmx` command calls this API in M2. It is there for M3's `apply --follow` and for
    debugging with grpcurl.
- **Client.**
  - `lmx status` gives the owner 2 seconds to connect and answer. Otherwise `owner` is `null`
    with an `owner` problem, and the exit status stays 0.
  - For `lmx store reserve`, a missing socket or a refused or timed-out connection becomes
    `owner.unavailable` (exit 3). The call itself has no deadline.

## Contract v1 additions

All of these are compatible additions; see "Change the host contract" in `docs/contract.md`.

- **`lmx store reserve`**, for root only.
  - `data`: `before`, `after`, `freed_bytes` and `collected`. `before` and `after` are disk
    usage objects, as in `status`.
  - Errors: `disk.low`, `disk.unreadable`, `permission.denied` and `owner.unavailable`.
    `disk.low` has `details`: `{before, after, freed_bytes, collect_error?}`.
  - Exit statuses:
    - 0 on success;
    - 1 for `disk.low`, `disk.unreadable` and `permission.denied`;
    - 2 for a usage error;
    - 3 for `owner.unavailable`.
  - Without `--json`, the command prints a sentence for people, built from the same values.
- **`disk.unreadable`.** The usage of the store file system cannot be read.
- **`status.data.owner`.** It is `null` when `lmxd` cannot be asked. Otherwise it is
  `{version, conditions: [{type, message}], operations: [{task, kind, phase, created_at}]}`.
  `created_at` is when the operation was requested, in Unix milliseconds.
- **Examples.**
  - New: `contract/v1/store-reserve.json` and `contract/v1/store-reserve-disk-low.json`.
  - `status.json` gains `owner`.
  - `status-partial.json` shows `owner: null` and its problem.

## Daemon lifecycle

- **Command line.** `lmxd [--config PATH] [--socket PATH]`. Without a readable configuration
  it exits with an error, and systemd restarts it.
- **Socket.** It comes from `LISTEN_FDS`. Otherwise `lmxd` binds the path and sets mode `0666`
  itself; M3's transient daemon works this way.
- **Readiness.** `READY=1` once the server listens.
- **Watchdog.** A ping every `WATCHDOG_USEC / 2`. Missed ticks are skipped.
- **SIGTERM:**
  1. send `STOPPING=1`;
  2. stop accepting connections;
  3. shut the supervisor down concurrently with the drain, bounded by 5 seconds;
  4. shut down the subprocess runner.

  A running collection is cancelled with its process group. Nix's GC is safe to interrupt.
- **Logging.**
  - When `JOURNAL_STREAM` is set, logs go to journald through `solti-observe`. Otherwise they go
    to standard error as text.
  - Task output reaches the journal through the output sink.
  - Field names keep the `F_` prefix of tracing-journald until M4.
- **Reference units** in `packaging/systemd/`, for M1c to render:
  - `lmx.socket`: `ListenStream=/run/lmx/lmx.sock`, `SocketMode=0666`,
    `WantedBy=sockets.target`.
  - `lmx.service`: `Type=notify`, `WatchdogSec=30s`, `Restart=on-failure`,
    `KillMode=control-group`, `LimitCORE=0`, `WantedBy=multi-user.target`. The service
    starts at boot because the guard must run.

## Testing

The test set is small on purpose. The goal is a working daemon on solid rails.

- **Unit tests** for pure logic:
  - `below`, with the cases of the Go and guard tests: healthy, low, critical, and no inode
    table;
  - the first tick, from uptime;
  - the commands of the task kinds, and the roots filter with the real `grep`;
  - configuration checks at startup;
  - proto conversions;
  - the authorization matrix.
- **Integration tests.** They run in process, with fake `nix-store`, `nice`, `ionice` and
  `grep` scripts and a scripted usage source:
  1. two concurrent reserves below 20% run one collection, and both succeed;
  2. a reserve that stays below 10% returns `disk.low` and runs `StoreRoots`;
  3. the guard, on a low disk, collects through `nice` and `ionice`;
  4. `lmx status` works with a running `lmxd` and without one, and `lmx store reserve` without one
     answers `owner.unavailable` with exit status 3.
- **Guest check** in the personal LimaNix VM. As in S1, it uses transient units and the real
  `nix-store`, and removes everything afterwards.

## Delivery

- `release/build` puts `lmxd` next to `lmx` in `lmx-<version>-<system>/`.
- Solti comes from the local SDK by path until 0.0.7 is published. The SDK is released before
  `lmx`.
- Containerized checks mount the SDK at the same path.
- Documentation to update: README, ARCHITECTURE, `docs/contract.md`, and the status line of the
  guest owner design.

## Decisions

| Question | Decision |
|---|---|
| `lmx` commands in M2 | Only what the host needs: `lmx store reserve`, and owner state in `lmx status` |
| Reserve when the disk stays low | Error `disk.low` with usage details. The host keeps treating it as a warning. |
| Shape of the store domain | `StoreCollect` and `StoreRoots` are Solti tasks. The guard and reserve are daemon logic that start them. |
| Guest checks | In the personal LimaNix VM, with transient units, cleaned up afterwards |
| Test scope | Unit tests of pure logic, plus three integration scenarios |
