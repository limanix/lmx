# M4: doctor, net check, logs, short status and theme in `lmx`

Status: design agreed on 2026-10-07 and implemented in this repository; see
[the M4 plan](2026-10-07-m4-guest-tools.md). It refines M4 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 5, 9, 10 and 12) for this
repository; the catalog and platform parts move to M1c.

## Scope

In M4, in this repository:

- **`lmx doctor [--json]`:** findings about the configuration, `lmxd` and the generations, with a
  hint for the next step.
- **`lmx net check PORT [--udp] [--json]`:** the guest side of the "Check a connection in order"
  guide: firewall rule and protocol, listener and bound address, process.
- **`lmx status --short`:** the words that need attention, for tmux and the prompt.
- **`lmx logs KIND [--previous]`:** the output of the latest run of an `lmxd` task kind, read from
  journald, also across a reboot.
- **Journal fields of `lmxd`:** `LMX_TASK`, `LMX_KIND` and `LMX_GENERATION`, without the `F_`
  prefix of tracing-journald.
- **Theme:** `lmx` colors its output from the palette in its configuration instead of the built-in
  Catppuccin Mocha.

Not in M4 (M1c):

- the `_shared/theme.nix` capability in the catalog and the move of every module to it;
- rendering the new configuration fields into `/etc/lmx/config.json`;
- the tmux and Starship segments that call `lmx status --short`;
- a journald size limit in the platform base;
- `limanix doctor` and `limanix net check` on the Mac, which add the VM state, the guest address and
  a TCP attempt from the Mac.

## Current state

- **Theme.** Catppuccin Mocha is wired separately in every place that has colors:
  - tmux, lazygit, yazi and Starship read `_shared/palette.toml` and pick the `mocha` table;
  - astronvim, harlequin and posting name the theme;
  - the platform's `welcome.sh`, its fallback prompt and `lmx`'s `palette.rs` repeat the colors as
    RGB.

  Nothing in the declaration selects a theme.
- **Diagnostics.** Neither the client nor `lmx` has a doctor or a network check. The steps exist as
  the guide [Check a connection in order](https://limanix.dev/categories/client/networking.html):
  1. the VM runs and has an address;
  2. a shell opens;
  3. the application listens on the expected port and a reachable interface;
  4. the firewall opens the port for the right protocol;
  5. after a port change, `limanix update` runs;
  6. the application's own access rules allow the client.
- **Ports.** `network.ports {tcp, udp}` in `limanix.toml` becomes
  `networking.firewall.allowedTCPPorts` and `allowedUDPPorts`; the firewall is iptables with the
  chain `nixos-fw`. Lima forwards no ports. Docker publishes ports with its own rules, outside
  `network.ports`.
- **Journal.** It is persistent, and nothing limits its size. `lmxd` writes task output with the
  fields `F_LMX_TASK` and `F_EXIT_CODE`; the `F_` prefix is tracing-journald's default. The dev user
  is in neither `wheel` nor `systemd-journal`, so it cannot read the system journal.

## Components

| Crate | Change |
|---|---|
| SDK `solti-observe` | `LoggerConfig.journald_field_prefix: Option<String>`, default `Some("F")`, so the behavior of other users does not change |
| `lmx-model` | Configuration fields `network.ports`, `theme` and `tools.journalctl`; the check record and the answers of `doctor` and `net check` |
| `lmx-facts` | Listening sockets from `/proc/net`, their owners from `/proc/*/fd`, journal records from `journalctl -o json` |
| `lmxd` | Journal fields without a prefix: `LMX_TASK` and `LMX_KIND` on task output, plus `LMX_GENERATION` on apply events |
| `lmx` | `doctor`, `net check`, `status --short`, `logs`; the palette from the configuration |

All four commands are facts: they run in the caller's process with the caller's privileges, and
`doctor` and `logs` work while `lmxd` is down, when they are needed most.

**Configuration.** Schema 1 gains these fields; M1c renders them:

- `network.ports.tcp` and `network.ports.udp`: the declared ports, as the firewall opens them;
- `theme.flavor` and `theme.palette`: the flavor's table of `palette.toml`, color names to
  `#rrggbb`;
- `tools.journalctl`: an absolute store path.

**Privileges.** As root (`sudo lmx …`) every answer is complete. Without root:

- `doctor` cannot read the marker of the mounted generation;
- `net check` cannot see the processes of other users;
- `logs` cannot read the system journal.

Such parts are marked `unknown` with a hint to run the command with `sudo`; they do not fail it.

## Checks

`doctor` and `net check` answer with check records:

```json
{"check": "owner", "status": "warning", "message": "…", "hint": "…"}
```

| Status | Meaning |
|---|---|
| `ok` | The check passed. |
| `warning` | Something needs attention, but nothing is broken. |
| `failed` | Something is broken; `hint` says what to do. |
| `unknown` | The caller lacks the privileges to check; `hint` says to use `sudo`. |

- **Text.** Aligned rows: the status, colored from the theme, the check and the message; the hint
  goes on its own line below.
- **JSON.** The contract envelope with `data.checks`; `ok` is `true` whenever the command ran.
- **Exit status.** 1 when any check failed, otherwise 0. A warning never fails the command, and
  neither command uses exit status 3.

## `lmx doctor`

1. **`config`.** `/etc/lmx/config.json` is readable and valid. If not, `failed`: the platform
   renders it, and `limanix update` fixes it.
2. **`owner`.**
   - `lmxd` answers `Owner.Status` within 2 seconds: `ok`, with its version.
   - Its version differs from `lmx`: `warning`, restart the VM.
   - It does not answer: `failed`, with `systemctl status lmx.socket lmx.service` and
     `journalctl -u lmx` as hints.
3. **`generations`.** From the conditions of `lmxd`; without `lmxd`, from the generation markers:
   - `OutOfDate`: `warning`, the host builds the generation with `limanix update`; while a
     `SystemApply` runs, an update is in progress;
   - `RestartRequired`: `warning`, restart the VM, which `limanix update` does;
   - `Degraded`: `failed`, with the reason and `systemctl status <unit>` as a hint;
   - `DiskLow`: `warning`, with `sudo lmx store reserve` as a hint;
   - `Converged`, a pending finalize, or a system without markers (built before `lmx`): `ok`.

## `lmx net check PORT [--udp]`

The protocol is TCP unless `--udp` is given.

1. **`firewall`.** Whether PORT is in `network.ports.tcp` (or `udp`).
   - Only in the other protocol: `failed`, the port is open for the other protocol only.
   - In neither: `failed`, add it to `network.ports` in `limanix.toml` and run `limanix update`.
   - TCP 22: `ok`, SSH is always open.
   - A port that `docker-proxy` holds: `ok`, Docker publishes it with its own rules, outside the
     guest firewall.
2. **`listener`.** The sockets on PORT in `/proc/net/{tcp,tcp6}` in state `LISTEN`, or in
   `/proc/net/{udp,udp6}`:
   - none: `failed`, nothing listens;
   - only `127.0.0.1` or `::1`: `failed`, the port is reachable only inside the guest; listen on
     `0.0.0.0`;
   - `0.0.0.0`, `::` or another address: `ok`.
3. **`process`.** The pid and command that hold the listening socket, found by its inode in one
   search of `/proc/*/fd`, and the user that created the socket.
   - A process of another user is invisible without root: `unknown`.
   - No process holds it, and every process was searched: `ok`, the kernel does.
   - `docker-proxy`: `warning`, Docker publishes the port outside `network.ports`, and removing it
     from `network.ports` does not close it.

The answer is `{"port": 8080, "protocol": "tcp", "checks": [...]}`.

## `lmx status --short`

- **Source.** `Owner.Status` from `lmxd`, with 200 ms for the connection and the answer together.
  There is no fallback to facts: the dev user cannot read the marker of the mounted generation.
- **Words**, in this order:
  - `degraded`;
  - `disk-low`;
  - `restart`;
  - `applying` while a `SystemApply` runs, otherwise `apply` for `OutOfDate`;
  - `lmxd?` when `lmxd` does not answer.
- **Output.** The words separated by spaces, or nothing when all is well. The exit status is always
  0, so a prompt never breaks.
- **Usage.** It cannot be combined with `--json` or `--wait`. Colors belong to the tmux and Starship
  configuration of the catalog (M1c).

## `lmx logs KIND [--previous]`

KIND is one of `apply`, `health`, `finalize`, `collect` and `roots`: `SystemApply`, `SystemHealth`,
`SystemFinalize`, `StoreCollect` and `StoreRoots`.

- **Reading.** `journalctl -o json --no-pager --all -b 0` (`-b -1` with `--previous`) with the
  matches `_UID=0` and `LMX_KIND=<Kind>`: any user may write a record with `LMX_KIND`, but only root's
  count. A run is a task name of one process (`_PID`), because a restarted `lmxd` numbers its tasks
  from 1 again; the latest run is printed.
- **Output.**
  - A header with the task, the generation when it has one, and the time.
  - Then the task's lines.
- **Failures,** each with exit status 1:
  - no run, also when the journal has no previous boot: no run of the kind in this boot, try
    `--previous`;
  - an unreadable journal, which journalctl reports with a hint or by failing: run
    `sudo lmx logs KIND`.
- **Contract.** Text only. The host follows an apply live, so `logs` is not part of the host
  contract.

**Journal fields of `lmxd`.**

- `lmxd` starts its logger with no field prefix, through the new SDK option.
- Output lines of tasks carry `LMX_TASK` and `LMX_KIND`.
- Apply events carry `LMX_TASK`, `LMX_KIND` and `LMX_GENERATION`: the start, the build's start and
  the outcome. An apply is named when it starts, so one that fails before its build is a run too.
- Health and finalize events carry the same fields: a failed check, also one that failed before its
  task, and the outcome of a finalize.
- Other fields lose the prefix too: `EXIT_CODE`, `ERROR`.

## Theme

- **Source.** `config.theme.palette`.
- **Colors by role:**

  | Color | Used for |
  |---|---|
  | `blue` | commands and the logo |
  | `mauve` | the second part of the logo |
  | `subtext0` | secondary text |
  | `overlay1` | labels |
  | `green` | `ok`, read-write folders |
  | `peach` | read-only folders |
  | `yellow` | `warning` |
  | `red` | `failed` |
- **Fallbacks.** A missing or invalid color takes Mocha's value; without a configuration, `lmx`
  uses Mocha.
- **When to color:** as before. Only on a terminal, never with `NO_COLOR` or `TERM=dumb`.

## Contract v1 additions

All of these are compatible additions.

- **Commands:** `lmx doctor --json` and `lmx net check PORT [--udp] --json`.
- **Data:** check records with the statuses `ok`, `warning`, `failed` and `unknown`.
- **Examples:** `contract/v1/doctor.json` and `contract/v1/net-check.json`.

## Testing

The test set is small on purpose.

- **Unit tests:**
  - the `/proc/net` parser: IPv4 and IPv6, TCP `LISTEN`, UDP;
  - the decisions of `net check`;
  - the checks `doctor` derives from conditions;
  - the words of `--short`;
  - the latest run from journal JSON;
  - the palette from the configuration, with its fallback;
  - the prefix option in the SDK.
- **Integration tests,** in the existing in-process harness:
  - `doctor` without `lmxd` (`owner` failed, exit status 1) and with it (`RestartRequired` gives a
    warning);
  - `status --short` with `lmxd` (`restart`) and without it (`lmxd?`);
  - `net check` on a prepared `/proc` (a loopback-only listener fails);
  - `logs` with a fake `journalctl`.
- **Guest check, in the personal VM, harmless parts only:**
  - `doctor` as root and as dev;
  - `net check 22` (sshd) and a port that is not declared;
  - `status --short`;
  - the `LMX_*` fields in the journal after a transient daemon ran.

## Decisions

| Question | Decision |
|---|---|
| Scope | This repository only; the catalog, the platform and the host commands come with M1c |
| Checks of `doctor` | Core only: configuration, `lmxd`, generations and conditions |
| `status --short` | Only what needs attention, no colors, 200 ms |
| Where the commands run | In `lmx`, as facts; `lmxd` only changes its journal fields |
| Firewall rule of `net check` | The ports in the configuration: one generation renders them and the firewall, so they cannot drift. M1c renders the evaluated `networking.firewall` lists, so ports that modules open count; Docker's ports pass its own rules |
| Journal field names | No prefix, through an option in `solti-observe` |
