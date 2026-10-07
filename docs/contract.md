# Host contract

The LimaNix host runs `sudo lmx <command> --json` over management SSH and reads one JSON answer from standard output.
This page defines contract version 1.

## Read one answer

Every `--json` answer is a single line with the same envelope:

```json
{"contract": 1, "ok": true, "data": {}}
{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {}}}
```

| Field      | Meaning                                                                                 |
| ---------- | --------------------------------------------------------------------------------------- |
| `contract` | Contract version. A host that does not know the version must not decode the rest.       |
| `ok`       | `true` selects `data`; `false` selects `error`.                                         |
| `data`     | Command-specific result.                                                                |
| `error`    | `code` for programs, `message` for people, optional `details` for code-specific values. |

A host decodes an answer in this order:

1. Read `contract`. If the version is unknown, stop: the rest cannot be decoded.
1. Read `ok`. An answer with `ok: true` and no `data`, or with `ok: false` and no `error`, is a protocol error.
1. Treat an unknown error `code` as a generic failure and show its `message`.
1. Keep `[]` and `null` apart: an empty list is a fact that was read, and `null` is a fact that was not.

A command that writes no answer, for example after a usage error or a lost connection, has failed.

## Interpret the exit status

| Status | Meaning                                                                                 |
| ------ | --------------------------------------------------------------------------------------- |
| `0`    | Success                                                                                 |
| `1`    | The operation failed; see the JSON answer, or standard error when no answer was written |
| `2`    | Usage error                                                                             |
| `3`    | The guest owner daemon is unavailable                                                   |
| `130`  | The apply was cancelled (`apply.cancelled`)                                             |

## Error codes

| Code                       | Meaning                                                            |
| -------------------------- | ------------------------------------------------------------------ |
| `owner.unavailable`        | `lmxd` is not reachable                                            |
| `apply.build_failed`       | `nixos-rebuild` failed for the requested generation                |
| `apply.cancelled`          | The operation was cancelled by an explicit request                 |
| `apply.environment_failed` | The environment files of the generation could not be installed     |
| `disk.low`                 | Free bytes or inodes are below the platform minimum                |
| `disk.unreadable`          | The usage of the store file system cannot be read                  |
| `network.unreachable`      | A required destination, such as the binary cache, is unreachable   |
| `permission.denied`        | The caller is not allowed to run the operation                     |
| `generation.mismatch`      | The mounted inputs belong to a different generation than requested |
| `contract.unsupported`     | The caller requested a contract version this binary does not speak |
| `system.degraded`          | The booted generation failed its health check                      |
| `wait.timeout`             | A wait ended before its condition held                             |

`lmx status` without `--wait`, `lmx version`, `lmx doctor` and `lmx net check` do not use these codes or the exit statuses `3` and `130`.
Owner operations, `lmx store reserve`, `lmx apply`, `lmx apply cancel` and `lmx status --wait`, use exit status `3` when `lmxd` is unavailable.
Only `lmx apply` uses exit status `130`.

## `lmx status`

`data` describes the guest. Every fact is optional: an unreadable fact is `null` and its reason is listed in `problems`.

| Field          | Meaning                                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `generations`  | `desired` (mounted at `/mnt/limanix`), `built` (system profile), `booted` (running system)                                |
| `disk`         | `bytes`, `free_bytes` (including the root reserve), `available_bytes`, `inodes`, `free_inodes`                            |
| `interfaces`   | `name`, lowercase `mac` (`null` without a hardware address), and global-scope `ipv4` addresses of each interface          |
| `failed_units` | Names of failed systemd units                                                                                             |
| `owner`        | The guest owner daemon: `version`, `conditions` (`type`, `message`), `operations` (`task`, `kind`, `phase`, `created_at`) |
| `problems`     | `fact` and `message` for each fact that could not be read in full; omitted when empty                                     |

A problem's `fact` names the field that is `null` or incomplete, or is `config` when `/etc/lmx/config.json` cannot be read.
`owner` is `null` with an `owner` problem when `lmxd` does not answer within two seconds; the other facts are still answered.
Its conditions are computed when it is asked:

| Condition         | When                                                                                                       |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `OutOfDate`       | The mounted generation is not built: an apply is needed                                                    |
| `RestartRequired` | The mounted generation is built but not booted: a restart is needed                                        |
| `Degraded`        | The mounted generation is booted and failed its last health check; the message gives the reason            |
| `Converged`       | The mounted generation is booted, healthy and finalized: older generations and their boot entries are gone |
| `DiskLow`         | Less than the platform minimum of the store disk is free                                                   |

Without a mounted generation there is no generation condition.
A booted generation that is healthy and still keeps older generations has none either: `lmxd` is about to remove them, and a `SystemFinalize` operation shows it.
An operation's `created_at` is when it was requested, in Unix milliseconds.
Without the configuration, `ip` and `systemctl` are looked up in `PATH`, so a `config` problem marks a degraded answer.

A generation is `null` when its stage has no valid marker, for example on a system built before `lmx` existed.
A marker that exists but cannot be read also gives `null` and adds a `generations` problem.
The host compares the three generations to tell whether a build or a restart is still needed.

Examples: [complete](../contract/v1/status.json), [partial](../contract/v1/status-partial.json).

## `lmx store reserve`

Root only. The host runs it before it stops a running VM for an update.
`lmxd` collects unreferenced store paths when less than the collect threshold (20%) of the store disk is free, and waits for the collection; a collection already running is shared.
Interrupting the command does not stop the collection.

`data` has `before` and `after`, store disk usage objects as in `lmx status`, `freed_bytes`, and `collected`.

| Error code          | When                                                                                                                                                          |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `disk.low`          | Less than the platform minimum (10%) is still free afterwards; `details` has `before`, `after`, `freed_bytes`, and `collect_error` when the collection failed |
| `disk.unreadable`   | The usage of the store file system cannot be read                                                                                                             |
| `permission.denied` | The caller is not root                                                                                                                                        |
| `owner.unavailable` | `lmxd` cannot be reached; exit status `3`                                                                                                                     |

The host treats `disk.low` as a warning: the update may still succeed.

Examples: [enough room](../contract/v1/store-reserve.json), [disk low](../contract/v1/store-reserve-disk-low.json).

## `lmx status --wait converged -g GENERATION`

The host runs it after it restarts the VM into a new generation.
`lmx` asks `lmxd` every two seconds until it reports `Converged` and the booted generation is `GENERATION`, then answers as `lmx status` does.
A daemon that does not answer yet, as right after the restart, is waited for.
`--timeout` limits the wait, such as `90s`, `10m` or `1h`; the default is 10 minutes.

| Error code          | When                                                                       |
| ------------------- | -------------------------------------------------------------------------- |
| `system.degraded`   | At the timeout, the generation is `Degraded`; the message gives the reason |
| `wait.timeout`      | At the timeout, the generation is not converged for another reason         |
| `owner.unavailable` | `lmxd` never answered; exit status `3`                                     |

`details` of `system.degraded` and `wait.timeout` has `conditions`, the conditions `lmxd` reported last.

## `lmx apply -g GENERATION`

Root only. The host runs it after it mounts a generation at `/mnt/limanix`.
`lmxd` installs the generation's environment files into `/etc/limanix`, makes room in the store as `lmx store reserve` does, and builds the generation for the next boot with `nixos-rebuild boot`.
The apply belongs to `lmxd`: interrupting the command does not stop it, and running the command again attaches to the running apply.
An apply of another generation is cancelled and replaced.

Without `--follow`, the answer comes at once.
`data` has `generation` and `state`: `running`, or `restart_required` when the generation is built.

With `--follow`, the command waits for the outcome.
`--json` then writes JSON Lines: one line per event, and the envelope last.

| `event`   | Fields                                                                                                   |
| --------- | -------------------------------------------------------------------------------------------------------- |
| `phase`   | `phase`: `environment`, `reserve` or `build`                                                             |
| `output`  | `stream` (`stdout` or `stderr`) and `line`, a line of the build; `truncated: true` when the line was cut |
| `warning` | `code` and `message` of a problem that does not stop the apply, such as `disk.low` from the reserve      |
| `lagged`  | `skipped`: the follower fell behind and missed that many events                                          |

A follower that attaches to a running apply receives the events from then on, and always the outcome.
On success the envelope's `state` is `restart_required`: restart the VM to boot the generation.

| Error code                 | When                                                                                                                         |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `generation.mismatch`      | `GENERATION` is not the mounted generation; `details` has `requested` and `mounted` (`null` without a mount)                 |
| `apply.environment_failed` | The environment files cannot be installed                                                                                    |
| `apply.build_failed`       | `nixos-rebuild` failed; `details` has `exit_code` when it exited, and `disk` with the store disk usage when the disk is full |
| `apply.cancelled`          | `lmx apply cancel`, an apply of another generation, or a cancel through the Task API stopped the apply; exit status `130`    |
| `permission.denied`        | The caller is not root                                                                                                       |
| `owner.unavailable`        | `lmxd` cannot be reached, or it stopped before the apply ended; exit status `3`                                              |

Without `--json`, the command prints the build's lines on their own streams and a closing sentence.

Examples: [built](../contract/v1/apply-restart-required.json), [build failed](../contract/v1/apply-build-failed.json), [followed](../contract/v1/apply-follow.jsonl).

## `lmx apply cancel -g GENERATION`

Root only. The host runs it when the person stops an update.
`lmxd` cancels the apply of `GENERATION` and answers once it has stopped; its followers receive `apply.cancelled`.
A build is killed; a boot loader update that `nixos-rebuild` started finishes in its own unit.

`data` has `cancelled`: `true` when an apply of the generation was running, `false` when none was.

| Error code          | When                                      |
| ------------------- | ----------------------------------------- |
| `permission.denied` | The caller is not root                    |
| `owner.unavailable` | `lmxd` cannot be reached; exit status `3` |

## `lmx doctor`

`lmx doctor` diagnoses the guest owner and works without `lmxd`.
`data.checks` lists check records in the order the checks ran:

| Field     | Meaning                                              |
| --------- | ---------------------------------------------------- |
| `check`   | Stable name of the check, such as `owner`            |
| `status`  | `ok`, `warning`, `failed`, or `unknown`              |
| `message` | The finding, for people                              |
| `hint`    | What to do next; omitted when there is nothing to do |

`unknown` means the caller lacks the privileges to check, and the hint says to use `sudo`.
The exit status is `1` when any check failed; a warning does not fail the command.

| Check         | Finds                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------ |
| `config`      | Whether `/etc/lmx/config.json` is readable and valid                                                   |
| `owner`       | Whether `lmxd` answers, and whether its version is the version of `lmx`                                |
| `generations` | The generation conditions of `lmxd`, such as `RestartRequired`, or what the markers say without `lmxd` |
| `disk`        | `DiskLow`, when `lmxd` reports it                                                                      |

Example: [doctor](../contract/v1/doctor.json).

## `lmx net check PORT`

`lmx net check PORT [--udp]` checks why a port of the VM may be unreachable from the Mac.
`data` has `port`, `protocol` (`tcp` or `udp`) and `checks`, check records as in `lmx doctor`, with the same exit status.

| Check      | Finds                                                                                                                                                  |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `firewall` | Whether the guest firewall opens the port for its protocol; TCP 22 is always open for SSH, and Docker publishes ports with its own rules               |
| `listener` | Whether something listens on the port, and whether only on a loopback address                                                                          |
| `process`  | Which process holds the socket, and which user created it; Docker's proxy is a warning, because `network.ports` does not control what Docker publishes |

Without a listener there is no `process` check.
The VM's address and a connection from the Mac belong to `limanix net check` on the host.

Example: [net check](../contract/v1/net-check.json).

## `lmx version`

`data` has `version`, the release version of the binary, and `contract`, the contract version it speaks.

Example: [version](../contract/v1/version.json).

## Change the host contract

- Adding an optional field, a new command or a new error code is compatible and keeps the version.
  Hosts treat an unknown code as a generic failure.
- A new `event` of `lmx apply --follow` is compatible too: hosts skip events they do not know.
  A new phase or state needs a new contract version.
- Renaming, removing or changing the meaning of a field needs a new contract version.
- Every version keeps its examples in `contract/v<version>/`; tests decode and re-encode them without loss.
- The LimaNix client pins an `lmx` release and tests its decoders against that release's examples.
