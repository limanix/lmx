# Host contract

Version 1 of the JSON interface between the LimaNix client and `lmx`. The client
runs `sudo lmx <command> --json` over management SSH.

For command usage, see the [lmx guide](https://docs.limanix.dev/inside/lmx) and
[command reference](https://docs.limanix.dev/reference/lmx).

## Responses

Most commands return one JSON object on one line:

```json
{"contract": 1, "ok": true, "data": {}}
{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {}}}
```

- Check `contract` before decoding the payload; reject unsupported versions.
- `ok: true` requires non-null `data`; `ok: false` requires non-null `error`.
- Use `error.code` in code and `error.message` for display. `details` is
  optional.
- In status, `null` means unavailable; `[]` means a known empty list. A missing
  generation marker also gives `null`, without a problem record.
- A command that returns no envelope has failed.

`lmx apply --follow --json` returns an event stream followed by the final
response; see [Apply](#apply).

## Commands and examples

Payload types and error codes are defined in
[lmx-model](../crates/lmx-model/src/). The examples below are checked by Rust
tests.

| Command | Result | Examples |
| -- | -- | -- |
| `status` | Generations, disk, interfaces, failed units and daemon state | [Complete](v1/status.json), [partial](v1/status-partial.json) |
| `status --wait converged -g GENERATION` | Status once the requested generation is booted, healthy and finalized | [Finalize failed](v1/wait-finalize-failed.json) |
| `store reserve` | `before`, `after`, `freed_bytes`, `collected` | [Enough room](v1/store-reserve.json), [disk low](v1/store-reserve-disk-low.json) |
| `apply -g GENERATION` | `generation` and `state`: `running` or `restart_required` | [Built](v1/apply-restart-required.json), [build failed](v1/apply-build-failed.json), [mismatch](v1/apply-generation-mismatch.json), [followed](v1/apply-follow.jsonl) |
| `apply cancel -g GENERATION` | `cancelled`: whether a matching apply was running | [Cancelled](v1/apply-cancel.json) |
| `doctor` | `checks` with findings and hints | [Doctor](v1/doctor.json) |
| `net check PORT` | `port`, `protocol` and `checks` | [Port check](v1/net-check.json) |
| `version` | Binary `version` and `contract` version | [Version](v1/version.json) |

`doctor` and `net check` use the same check records: `check`, `status`,
`message` and optional `hint`. Status is `ok`, `warning`, `failed` or `unknown`.
Only `failed` makes the command exit with status 1; `unknown` means the caller
lacks permission to check. Their successful JSON envelope still carries the
findings in `data.checks`.

## Status and convergence

Compare `generations.desired` (selected by the host), `built` (system profile)
and `booted` (running system). Unreadable facts are reported in `problems`;
other facts remain available even when `lmxd` cannot be reached.

Generation conditions are `OutOfDate` (build needed), `RestartRequired` (restart
needed), `Degraded` (health check failed) and `Converged` (healthy and
finalized). `DiskLow` reports low store space.

Generation conditions may be temporarily absent while finalizing. Completion
requires an explicit `Converged`, not just the absence of failures.

`FinalizeFailed` means the booted generation is healthy but cleanup or boot
entry updates failed. The generation remains usable and `lmxd` retries later.
The wait command returns `finalize.failed` immediately for that generation.
Otherwise, an expired wait reports `system.degraded` or `wait.timeout`, with the
last conditions in `error.details.conditions`. If the daemon never answers, it
returns `owner.unavailable`.

## Apply

Root only. `apply` reads the requested generation under
`/mnt/limanix/GENERATION/`: `flake/runtime.json`, `environment` and
`environment.sh`. It checks the generation marker, validates the environment
files, reserves store space, runs `nixos-rebuild boot`, then installs the
environment files. After success, the host selects the generation by atomically
replacing the relative symlink `/mnt/limanix/current`; a VM restart boots it.

The operation belongs to `lmxd`: disconnecting does not cancel it. Applying the
same generation joins the running operation; applying another cancels and
replaces it. `apply cancel` waits for the matching operation to stop.

Apply is not atomic across the system profile, boot entries and environment
files. A failed or cancelled build leaves the installed environment unchanged,
but may already have changed the profile and boot entries. After a successful
build, environment installation completes even if cancellation arrives; an
installation failure is reported as `apply.environment_failed`. Reapplying the
generation repairs the profile, boot entries and environment.

With `--follow --json`, stdout is JSON Lines: events first, the final response
last. A follower joining a running operation receives subsequent events and the
outcome. Success returns `state: restart_required`.

| Event | Fields |
| -- | -- |
| `phase` | `phase`: `reserve`, `build`, `environment` |
| `output` | `stream`: `stdout` or `stderr`; `line`; optional `truncated: true` |
| `warning` | `code`, `message`; the operation continues |
| `lagged` | `skipped`: number of missed events |

## Store reserve

Root only. Collects unreferenced Nix store paths when free space is below the
platform's collection threshold, and waits for completion. Calls share an
existing collection; disconnecting does not stop it. `disk.low` means space is
still below the platform minimum afterwards; the host treats it as a warning
because the update may still succeed.

## Exit status

| Status | Meaning |
| -- | -- |
| `0` | Success |
| `1` | Operation failed or diagnostic checks found a failure; read the JSON response |
| `2` | Usage error |
| `3` | `lmxd` unavailable for `apply`, `apply cancel`, `store reserve` or `status --wait` |
| `130` | Apply cancelled (`apply.cancelled`) |

## Change the host contract

- Optional fields, new commands, error codes, events, condition types and
  operation kinds are compatible additions. Readers ignore unknown fields,
  events, condition types and operation kinds; an unknown error code is a
  generic failure whose message can be displayed.
- Renaming, removing or changing a field's meaning requires a new contract
  version. New apply phases or states also require a version change.
- Keep each version's examples in `contract/v<version>/` and update its tests.
  Rust tests decode and re-encode the examples without loss; the client tests
  its decoders against the release it pins.

Follow the [contribution guide](https://docs.limanix.dev/contributing/) for
cross-repository changes and release order.
