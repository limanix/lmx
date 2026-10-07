# M3: apply and finalize in `lmxd`

Status: design agreed on 2026-10-07 and implemented in this repository; see
[the M3 plan](2026-10-07-m3-apply-finalize.md). Host integration follows in M1c. It refines M3 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 6–9 and 12) and builds on
[M2](2026-10-07-m2-lmxd-store-design.md).

## Scope

In M3, in this repository:

- **`lmxd`:**
  - the apply operation, with the RPCs `Owner.Apply` (streaming) and `Owner.CancelApply`;
  - a `--transient` mode for the daemon that the host starts from the mounted generation;
  - in system mode, a generation observer that checks health and finalizes after boot;
  - generation conditions in `Owner.Status`.
- **`lmx`:**
  - `lmx apply -g G [--follow] [--json]`;
  - `lmx apply cancel -g G [--json]`;
  - `lmx status --wait converged -g G [--timeout DUR]`.
- **`lmx-model`, `lmx-ipc` and `lmx-facts`:** the contract types, RPCs, configuration fields and
  facts these need.

Not in M3 (M1c):

- `generation` in `runtime.json`;
- rendering `/etc/lmx/config.json`;
- the flake output `#lmx`, which carries the binaries and that generation's
  `etc/lmx/config.json`;
- the units in the platform base;
- switching the host to `lmx apply` and `lmx status --wait`, and removing `rebuild.go`, `prune.go`,
  `installEnvironment` and the `true` check from the client.

## Behaviour to keep

The host's guest steps today (client `internal/guest`):

1. **Reserve.** Its result is ignored.
2. **Environment.** The host runs:
   - `install -d -m 0755 /etc/limanix`;
   - `install -m 0644 /mnt/limanix/environment /etc/limanix/environment`;
   - the same for `environment.sh`.

   The files come from the host's `[env]` and stay out of the flake and the Nix store.
3. **Build.** The host runs:

   ```
   systemd-run --unit=limanix-rebuild-<random> --wait --pipe --collect --quiet \
     --service-type=oneshot --property=KillMode=control-group \
     --property=TimeoutStartSec=infinity --property=TimeoutStopSec=5s --setenv=PATH \
     /run/current-system/sw/bin/nixos-rebuild boot --flake path:/mnt/limanix/flake#runtime \
     --no-write-lock-file --no-update-lock-file
   ```

   - The build has no timeout.
   - The unit also gets `EnvironmentFile=-/etc/limanix/environment` from the platform's
     drop-in for every service. User variables such as proxies or `NIX_CONFIG` therefore reach the
     build, and they override the `PATH` that `--setenv` passes.
   - On Ctrl-C the host runs `systemctl stop` on the unit: SIGTERM to the control group, then
     SIGKILL after 5 seconds.
4. **Restart and ready check.** The host restarts the VM, then runs a command as the dev user:

   ```
   sudo --set-home --user <user> -- bash --login -c 'cd -- "$HOME" && exec "$@"' limanix-command true
   ```
5. **Prune.** After the ready record, the host runs these in order, and a failure only warns:
   1. `nix-env --profile /nix/var/nix/profiles/system --delete-generations old`;
   2. if that worked, `/nix/var/nix/profiles/system/bin/switch-to-configuration boot`;
   3. always, `nix-store --gc --quiet`.

The host has no resume. Every update builds a new generation, and a host that dies leaves the build
unwatched. A failed environment install or build is explained as a full disk when usage is below the
minimum or the error contains `No space left on device`.

nixos-rebuild-ng runs `switch-to-configuration` in its own transient unit. Killing the build's
process group therefore leaves the boot loader update to finish.

## Components

| Crate | Change |
|---|---|
| `lmx-model` | Apply events and results; condition names; codes `apply.environment_failed`, `system.degraded`, `wait.timeout`; `Config.user.gid`, `Config.tools.{nixos_rebuild, nix_env, sudo, bash}`, `Config.health.units` |
| `lmx-facts` | The generations of the system profile (`system-N-link`) |
| `lmx-ipc` | `Owner.Apply` (server streaming), `Owner.CancelApply`, generation conditions in `Owner.Status` |
| `lmxd` | Apply operation, `--transient`, generation observer, health check and finalize, a system root for paths |
| `lmx` | `lmx apply`, `lmx apply cancel`, `lmx status --wait` |

**Configuration.** M1c renders these new fields. The transient daemon reads them from the
generation's package:

- `user.gid`: the dev user's primary group. It owns the environment files and is applied by number,
  so it works on the first create, before the platform exists.
- `tools.nixos_rebuild`, `tools.nix_env`, `tools.sudo`, `tools.bash` and `tools.systemd_run`:
  absolute store paths.
- `health.units`: the platform units the health check requires, such as `sshd.service`,
  `lima-guestagent.service` and `lmx.socket`.

**Tasks.** New `lmx.limanix.dev/v1` kinds, built the M2 way through the private subprocess runner:

| Kind | Runs | Slot |
|---|---|---|
| `SystemApply` | `nixos-rebuild boot --flake path:/mnt/limanix/flake#runtime --no-write-lock-file --no-update-lock-file` | `system`, queued |
| `SystemHealth` | the health script (below) | `health` |
| `SystemFinalize` | `nix-env --profile /nix/var/nix/profiles/system --delete-generations old`, then, if that worked, `switch-to-configuration boot` in its own unit through `systemd-run`, as `nixos-rebuild` runs it | `system`, dropped while a build runs |
| `StoreCollect`, `StoreRoots` | as in M2 | `store` |

**Environment of `SystemApply`.** It starts with `PATH=/run/current-system/sw/bin`. The variables
of the installed `/etc/limanix/environment` are added on top, so a user's `PATH` wins, as with
systemd's `EnvironmentFile` today.

**Task output.** `lmxd` captures the output of its own tasks without loss. A tee on the runner's
output publisher feeds apply followers and health-check reasons. Solti's live output stream starts
too late for these uses and has no replay.

## Conditions and health

`Owner.Status` derives conditions on every call. Its inputs:

- the desired, built and booted generation markers (`lmx-facts`);
- the generations of the system profile;
- in system mode, the result of the last health check.

| Condition | When |
|---|---|
| `OutOfDate` | desired is known and differs from built: an apply is needed |
| `RestartRequired` | desired equals built, booted differs |
| `Degraded` | desired, built and booted are equal, and the last health check failed. The message holds the reason, and older generations stay for a rollback. |
| `Converged` | desired, built and booted are equal, the system is healthy, the profile has one generation, and no finalize runs or waits for a retry |
| `DiskLow` | as in M2 |

- **Finalize pending.** Desired, built and booted equal and healthy, but with older generations,
  means finalize is pending. No generation condition is set, and `SystemFinalize` shows in
  `operations`.
- **Unknown markers.** If a marker cannot be read, there is no generation condition. This covers a
  system built before `lmx`.
- **Failed units.** Units that failed outside `health.units` stay visible in the existing
  `failed_units` fact of `lmx status`, and they never block finalize. There is no separate
  condition: `lmxd` runs programs only as tasks, so computing one on every status call is not
  worth it.

**Health check.** It runs only in system mode, as a `SystemHealth` task, while desired, built and
booted are equal and finalize is pending or the system is `Degraded`.

- **Schedule.** The first check runs 30 seconds after `lmxd` starts, then every minute. After
  `Converged` there are no more checks until the next boot: the check judges an update, it is not
  monitoring.
- **What it checks:**
  1. every unit in `health.units` is active, with `systemctl is-active`;
  2. `/mnt/limanix` and the dev user's home are mounted, read from `mountinfo` in process;
  3. the dev user can run a command, exactly as the host checks today:
     `<sudo> --set-home --user <user> -- <bash> --login -c 'cd -- "$HOME" && exec "$@"' limanix-command true`.
- **Result.** Exit status 0 means healthy. Otherwise the task's output gives the reason, such as
  `sshd.service is not active`. A check that runs longer than 2 minutes, such as on a stuck mount,
  fails.

## Apply

`Owner.Apply {generation, follow}` is for privileged callers only.

1. **Checks,** in this order:
   - mounted desired differs from G, or is unknown: `generation.mismatch`, with details
     `{requested, mounted}`;
   - an apply of G is running: the caller attaches to it;
   - an apply of another generation is running: `lmxd` cancels it, waits until it stops, and checks
     again. The host mounted another generation, so the older build must not finish after the answer,
     even when G is built;
   - built equals G: the result is `restart_required` at once;
   - otherwise a new apply starts. Its build queues in the `system` slot, so a finalize that runs
     there finishes first instead of being killed. Right before the build, the mounted generation is
     read again; one that changed is a mismatch.
2. **Phases:**
   - `environment`: copy `/mnt/limanix/environment` and `environment.sh` atomically into
     `/etc/limanix/`. The directory is `0755`; the files are `0640`, owned by root and the group
     `user.gid`. A failure is `apply.environment_failed`.
   - `reserve`: M2's reserve at normal priority, sharing the active collection. A low disk becomes a
     `disk.low` warning event and the apply goes on, as the host ignores Reserve in Apply today.
   - `build`: the `SystemApply` task.
3. **Outcome:**

   | Outcome | Answer |
   |---|---|
   | success | `restart_required` |
   | non-zero exit | `apply.build_failed`, details `{exit_code}`. If usage is below the minimum or the output contains `No space left on device`, the details also hold the disk usage. |
   | cancelled by `lmx apply cancel`, by an apply of another generation, or through the Task API | `apply.cancelled`, exit status 130 |
   | `lmxd` stops before the apply ends | `owner.unavailable`, exit status 3 |

- **State.** It is in memory and holds one current apply: G, its phase, the task, a cancellation
  token, and an event channel for followers. Nothing is stored on disk; after a reboot the truth comes
  from the markers again.
- **Disconnects.** A client that disconnects does not stop the apply. Running the same command again
  attaches: events continue from that point, and the outcome always arrives.

**Streaming.** `--json --follow` writes JSON Lines, ending with the contract envelope:

```json
{"event":"phase","phase":"environment"}
{"event":"warning","code":"disk.low","message":"…"}
{"event":"phase","phase":"build"}
{"event":"output","stream":"stderr","line":"building the system configuration..."}
{"event":"lagged","skipped":12}
{"contract":1,"ok":true,"data":{"generation":"0123456789ab","state":"restart_required"}}
```

- `output` events carry `"truncated": true` for a line that was cut.
- Without `--json`, the command prints the output lines and a final sentence.
- Without `--follow`, the command starts or attaches and answers at once with
  `{"generation": G, "state": "running"}`, or with `restart_required`.

**Cancel.** `Owner.CancelApply {generation}` is for privileged callers only.

- During the build it cancels the task. nix-daemon stops the build, and a running
  `switch-to-configuration` finishes in its own unit.
- In the environment or reserve phase, the apply stops between steps.
- The answer is `{"cancelled": true}`, or `false` when nothing was running. Followers receive
  `apply.cancelled`.

## Finalize and waiting

**Finalize** runs only in system mode, never in `--transient`. The observer ticks every minute; the
first tick comes 30 seconds after start.

1. When desired, built and booted are equal and the profile has older generations, or the system is
   `Degraded`, the observer runs the health check.
2. If healthy, it runs `SystemFinalize`, then `StoreCollect` at idle priority. Garbage collection
   continues in the background: `Converged` needs the finalize, not the collection.
3. If not healthy, the system is `Degraded`. The older generations stay, and the check repeats every
   minute.
4. A failed finalize is logged and retried at most every 15 minutes, even after it removed the older
   generations, so the boot entries get rewritten. While a finalize runs or waits for its retry, the
   generation is not `Converged`.

**`lmx status --wait converged -g G [--timeout DUR]`.** The default timeout is 10 minutes.

- **Polling.** It polls `Owner.Status` every 2 seconds.
- **Success.** It succeeds with the full `status` answer when `Converged` holds and booted equals G.
- **Daemon not up yet.** An `lmxd` that does not answer yet, as right after a reboot, is waited for.
- **At the timeout:**
  - `Degraded` gives `system.degraded`, with the reason;
  - a daemon that never answered gives `owner.unavailable`, with exit status 3;
  - otherwise the answer is `wait.timeout`, with the current conditions.

## Contract v1 additions

All of these are compatible additions.

- **Commands:**
  - `lmx apply -g G [--follow] [--json]`;
  - `lmx apply cancel -g G [--json]`;
  - `lmx status --wait converged -g G [--timeout DUR]`.
- **New codes:** `apply.environment_failed`, `system.degraded`, `wait.timeout`. M1a already declared
  `apply.build_failed`, `apply.cancelled` and `generation.mismatch`.
- **Exit statuses:** 0 success, 1 failure, 2 usage, 3 owner unavailable, 130 `apply.cancelled`.
- **Conditions:** `status.owner.conditions` gains `OutOfDate`, `RestartRequired`, `Converged` and
  `Degraded`.
- **Examples:**
  - `contract/v1/apply-follow.jsonl`;
  - `contract/v1/apply-restart-required.json`;
  - `contract/v1/apply-build-failed.json`;
  - a `status` example with generation conditions.

## Daemon lifecycle

- **`lmxd --transient --config <package>/etc/lmx/config.json`:**
  - no store guard and no generation observer;
  - the daemon binds the socket itself, as in M2 without `LISTEN_FDS`;
  - it lives until the host stops it or the VM restarts.
- **One daemon at a time.** A daemon refuses a socket that another daemon still serves, so the
  host stops the system daemon before it starts a transient one, and two daemons never apply at
  once.
- **The system daemon** can also apply. A finalize reads the markers again after its health check,
  and it is dropped while a build holds the `system` slot: after that build, the older generations
  would include the booted one. A build queues behind a running finalize.
- **System tasks** run with `PATH=/run/current-system/sw/bin`, as the host's login shell had it.
- **System root.** For tests, `Options` gains a system root for every path; the binary takes it
  from the hidden flag `--root`:
  - `/mnt/limanix`;
  - `/nix/var/nix/profiles/system`;
  - `/run/booted-system`;
  - `/etc/limanix`;
  - `/proc/self/mountinfo`.

## Testing

The test set is small on purpose.

- **Unit tests:**
  - deriving conditions from desired, built, booted, generations and health;
  - the apply decision: mismatch, `restart_required`, attach, start;
  - installing the environment files: content and modes;
  - parsing the environment file for the build;
  - encoding JSON Lines;
  - the finalize decision and its back-off.
- **Integration tests.** They run in process, on a fixture tree with fake `nixos-rebuild`,
  `nix-env`, `switch-to-configuration`, `systemctl` and `sudo`:
  1. an apply succeeds: phases, output lines, `restart_required`, and the environment files are in
     place;
  2. a failing build gives `apply.build_failed` with its exit status;
  3. a cancel during the build gives followers `apply.cancelled` (exit status 130), and the cancel
     answers `cancelled: true`;
  4. desired, built and booted are equal with two profile generations: finalize removes the older
     one, and `status --wait converged` succeeds.
- **Guest check.**
  - **In the personal LimaNix VM, harmless parts only,** with transient units, cleaned up
    afterwards:
    - the `--transient` start with a configuration;
    - `SystemHealth`, which runs `systemctl is-active` and `true` as the dev user;
    - the conditions.

    A real apply or finalize there would change that VM's system. Without M1c it would also answer
    `generation.mismatch`, because the guest has no generation markers.
  - **In M1c,** the whole path runs in a separate, throwaway VM: update, restart, `Converged`.

## Known gaps

- A finalize that keeps failing is only logged and retried every 15 minutes. Meanwhile there is no
  generation condition, so `lmx status --wait converged` ends with `wait.timeout` after its timeout.
  M1c decides how the host reports it.

## Decisions

| Question | Decision |
|---|---|
| Configuration of the transient daemon | The generation's flake package `#lmx` carries `etc/lmx/config.json`. The host passes it with `--config`. `lmx` never reads `runtime.json`. |
| Mode of the environment files | `0640`, owned by root and the dev user's group (by gid) |
| Shape of apply and finalize | Daemon logic orchestrates the steps; programs are Solti tasks (`SystemApply`, `SystemHealth`, `SystemFinalize`, `StoreCollect`), as in M2 |
| Failed units outside the platform list | The existing `failed_units` fact, not a condition |
| Guest check | Harmless parts in the personal VM now; the full path in M1c on a throwaway VM |
