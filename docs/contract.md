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
| `130`  | Cancelled                                                                               |

## Error codes

| Code                   | Meaning                                                            |
| ---------------------- | ------------------------------------------------------------------ |
| `owner.unavailable`    | `lmxd` is not reachable                                            |
| `apply.build_failed`   | `nixos-rebuild` failed for the requested generation                |
| `apply.cancelled`      | The operation was cancelled by an explicit request                 |
| `disk.low`             | Free bytes or inodes are below the platform minimum                |
| `network.unreachable`  | A required destination, such as the binary cache, is unreachable   |
| `permission.denied`    | The caller is not allowed to run the operation                     |
| `generation.mismatch`  | The mounted inputs belong to a different generation than requested |
| `contract.unsupported` | The caller requested a contract version this binary does not speak |

`lmx status` and `lmx version` do not use these codes or the exit statuses `3` and `130`; later commands do.

## `lmx status`

`data` describes the guest. Every fact is optional: an unreadable fact is `null` and its reason is listed in `problems`.

| Field          | Meaning                                                                                                          |
| -------------- | ---------------------------------------------------------------------------------------------------------------- |
| `generations`  | `desired` (mounted at `/mnt/limanix`), `built` (system profile), `booted` (running system)                       |
| `disk`         | `bytes`, `free_bytes` (including the root reserve), `available_bytes`, `inodes`, `free_inodes`                   |
| `interfaces`   | `name`, lowercase `mac` (`null` without a hardware address), and global-scope `ipv4` addresses of each interface |
| `failed_units` | Names of failed systemd units                                                                                    |
| `problems`     | `fact` and `message` for each fact that could not be read in full; omitted when empty                            |

A problem's `fact` names the field that is `null` or incomplete, or is `config` when `/etc/lmx/config.json` cannot be read.
Without the configuration, `ip` and `systemctl` are looked up in `PATH`, so a `config` problem marks a degraded answer.

A generation is `null` when its stage has no valid marker, for example on a system built before `lmx` existed.
A marker that exists but cannot be read also gives `null` and adds a `generations` problem.
The host compares the three generations to tell whether a build or a restart is still needed.

Examples: [complete](../contract/v1/status.json), [partial](../contract/v1/status-partial.json).

## `lmx version`

`data` has `version`, the release version of the binary, and `contract`, the contract version it speaks.

Example: [version](../contract/v1/version.json).

## Change the host contract

- Adding an optional field, a new command or a new error code is compatible and keeps the version.
  Hosts treat an unknown code as a generic failure.
- Renaming, removing or changing the meaning of a field needs a new contract version.
- Every version keeps its examples in `contract/v<version>/`; tests decode and re-encode them without loss.
- The LimaNix client pins an `lmx` release and tests its decoders against that release's examples.
