# S1: Solti in the guest

Status: all three checks pass, including a run under systemd in a LimaNix guest. This
is the throwaway spike from section 12 of [the design](2026-10-06-guest-owner-design.md).
Its findings feed the M2 plan.

## Setup

- **Spike.** Two binaries, about 350 lines, kept outside the repository:
  - `s1d`, a daemon: Solti supervisor, subprocess runner, `solti-api` gRPC on a Unix
    socket, `SO_PEERCRED` authorization, socket activation, `sd_notify` and a watchdog;
  - `s1c`, a client: the generated `TaskServiceClient` over the same socket.
- **Solti.** The local SDK checkout by path, version 0.0.6 plus the fixes below.
  Features: `api-core-adapter`, `api-grpc`, `exec-subprocess`, `observe-journald`.
- **Toolchain.** Rust 1.90.0, `rust-lld`, the `ci/rust:1.90.0` image, release profile
  with LTO and stripping, as in `lmx`.
- **Versions.** tonic 0.14.6, prost 0.14.4, hyper 1.12.0, tokio 1.53.2, listenfd 1.0.2,
  sd-notify 0.4.5, tracing-journald 0.3.2.
- **Where the checks ran.**
  - Debian 12 containers. aarch64 runs natively; x86_64 runs under Rosetta.
  - For check 3, a Python harness first plays systemd: it passes the socket as fd 3
    with `LISTEN_FDS` and `LISTEN_PID`, sets `WATCHDOG_USEC`, and reads `NOTIFY_SOCKET`.
  - Then a LimaNix guest: NixOS, systemd 260.4, Linux 6.18 on aarch64, with transient
    units only. The guest was left as it was found.

## 1. Static musl build: passes after an SDK fix

| Binary | aarch64 | x86_64 |
|---|---|---|
| `s1d` | 5.0 MiB, static | 5.8 MiB, static-pie |
| `s1c` | 1.4 MiB, static | 1.6 MiB, static-pie |

- **The blocker was in `solti-exec`.** It used `libc::__rlimit_resource_t`, which exists
  only for glibc and uClibc. Fixed in the SDK; see [SDK changes](#sdk-changes).
- **No C dependencies at runtime.** The normal and build dependency tree has no `ring`,
  `cc`, OpenSSL or aws-lc. `ring` reaches the SDK only through `rcgen`, a dev-dependency
  of the `solti` facade. So `cargo clippy --all-targets` of the facade for musl needs a
  musl C compiler, but consumers do not.
- **Size.** The journald logger from `solti-observe` (tracing-subscriber with
  `EnvFilter`) adds about 1 MiB to the daemon.

## 2. gRPC over a Unix socket with `peer_cred`: passes

Authorization needs no tower layer and no SDK change:

1. A tonic `Interceptor` reads `UdsConnectInfo::peer_cred` and puts an `ApiIdentity`
   into the request extensions: subject `uid:<uid>`, attributes `uid`, `gid`, `pid`.
2. With no authenticator configured, `solti-api` takes that identity from the
   extensions.
3. An `ApiAuthorizer` sees the `TaskOperation` and decides. This is simpler than a
   tower layer, which would have to map gRPC paths to operations.

```rust
impl Interceptor for PeerIdentity {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let credentials = request
            .extensions()
            .get::<UdsConnectInfo>()
            .and_then(|info| info.peer_cred)
            .ok_or_else(|| Status::unauthenticated("peer credentials are unavailable"))?;
        let identity = ApiIdentity::for_subject(format!("uid:{}", credentials.uid()))
            .with_attribute("uid", credentials.uid().to_string());
        request.extensions_mut().insert(identity);
        Ok(request)
    }
}

#[async_trait]
impl ApiAuthorizer for RootWrites {
    async fn authorize(&self, request: AuthorizationRequest<'_>) -> Result<(), ApiError> {
        let root = request.identity().and_then(ApiIdentity::subject) == Some("uid:0");
        match request.operation() {
            TaskOperation::Get
            | TaskOperation::List
            | TaskOperation::Watch
            | TaskOperation::ListRuns
            | TaskOperation::StreamLogs => Ok(()),
            _ if root => Ok(()),
            _ => Err(ApiError::Forbidden("only root may change tasks".into())),
        }
    }
}

// InterceptedService::new(GrpcApi::new(handler).with_authorizer(..).server(), PeerIdentity)
```

| Caller | list, get, runs | logs, watch (streams) | cancel, delete |
|---|---|---|---|
| root | yes | yes | yes |
| `nobody` | yes | yes | `PermissionDenied` |

- **Socket mode.** The socket must be `0666`; `SO_PEERCRED` decides what each caller may
  do. The socket unit sets `SocketMode=0666`. The transient daemon of section 7 binds the
  socket itself, so it must set the mode explicitly.
- **Streams over the socket work.** `StreamTaskLogs` delivers live lines, then
  `RunFinished`. `WatchTasks` delivers the current objects, then changes.
- **Cancel cleans up.** After `cancel`, no process of the task remains.
- **Exit codes.** A failed run carries its exit code and a message such as
  `execution failed: process exited with non-zero code: 3`. A successful run reports
  no exit code.

## 3. Socket activation and watchdog

### Protocol: passes

| Step | Result |
|---|---|
| Start with an inherited socket | The daemon takes fd 3 from `LISTEN_FDS` and sends `READY=1` |
| Watchdog, `WATCHDOG_USEC=2s` | Five pings in 4.5 s, every 1.0 s |
| Daemon frozen with `SIGSTOP` for 3 s | No pings; systemd would fire the watchdog after 2 s |
| SIGTERM | `STOPPING=1`; exit 0 within 30 ms |
| Call while no daemon runs | The call waits in the socket backlog. The next instance serves it with exit 0. |

### Real systemd in a LimaNix guest: passes

The units were transient:

```
systemd-run --unit=s1-spike --socket-property=ListenStream=/run/s1/s1.sock \
  --socket-property=SocketMode=0666 --property=Type=notify --property=WatchdogSec=4s \
  --property=Restart=on-failure --property=LimitCORE=0 \
  --property=Environment=S1_LOG=journald /tmp/s1/s1d
```

| Step | Result |
|---|---|
| After `systemd-run` | The socket is active; the service is inactive |
| First call | Activates the service. It reaches `running` after `READY=1` and the call is answered. |
| Callers | `nobody` lists; `nobody` cancel gets `PermissionDenied`; root sees the exit code 3 of a failed task |
| Live log stream | 8,319 lines in 2 s, contiguous, no `Lagged` |
| Daemon frozen for 7 s, `WatchdogSec=4s` | systemd aborts it and restarts it: `NRestarts=1`, new PID. Pings were every 2 s. |
| `systemctl stop` of the service | The socket stays active, and the next call starts a new instance |
| Stop with a log follower attached | 0.01 s; the follower sees `RunFinished`, then the stream ends |
| journald | The daemon's events and `solti-core`'s own (`TARGET=solti_core::supervisor`), with `F_LMX_TASK` and `F_LMX_GENERATION` |

Also seen:
- **The watchdog signal reaches the whole control group.** `SIGABRT` hit the daemon and
  the task shells. `systemd-coredump` handled them. `LimitCORE=0` kept it from storing
  cores.
- **Stopping only the service logs a warning,** because its socket is still active.
  This is expected. It is why step 1 of section 7 stops both `lmx.socket` and
  `lmx.service`.

## Findings for M2

1. **Shutdown order.** tonic's graceful shutdown waits for open streams. A
   `StreamTaskLogs` or `WatchTasks` stream ends only when the supervisor stops.
   - Draining the server before stopping the supervisor hangs. In the spike, a
     `logs --follow` client kept the daemon alive until it was killed after 15 s. Under
     systemd this would hold `systemctl stop lmx` for `TimeoutStopSec`.
   - **Rule for `lmxd`.** On SIGTERM:
     1. send `STOPPING=1`;
     2. stop accepting connections;
     3. shut the supervisor down concurrently with the drain, or before it;
     4. bound the drain with a timeout.

     The SDK example `agent_grpc.rs` already joins both. With that order the spike
     stops in about 4 ms, with a log follower and a watcher attached; both see the
     streams end.
2. **Slots and admission.** Tasks that share a slot are admitted one at a time. The
   default admission policy is `DropIfRunning`: a task that arrives at a busy slot ends
   `Canceled` and never runs. The slot plan in section 9 (`system` replace, `store`
   queue) must set the admission policy explicitly.
3. **Output is live only.** `StreamTaskLogs` on a finished task returns `NotFound`.
   - Some things need history: a `--follow` that attaches after the end,
     `lmx logs --previous`, and output after a restart. They need the journald output
     sink the design describes.
   - `solti-core` has the hook: `SupervisorApi::builder(..).with_output_sink(..)`.
4. **journald field names.** `solti-observe` installs tracing-journald with its default
   field prefix. The guest confirms that an event field `lmx_task` becomes
   `F_LMX_TASK`, not `LMX_TASK`. Either accept the prefix or add a prefix option to
   `solti-observe`.
5. **Watchdog ticks after a stall.** A tokio `interval` fires the missed ticks in a
   burst after a stall. Use `MissedTickBehavior::Skip` for the watchdog task.
6. **Kernel requirement.** On Linux, `solti-exec` marks descriptors close-on-exec with
   `close_range(CLOSE_RANGE_CLOEXEC)` (Linux 5.11 and later). The SDK documents this,
   and spawn fails without it.
   - LimaNix guests run 6.x kernels.
   - Rosetta does not emulate the call, so amd64 containers on an Apple Silicon Mac
     cannot run subprocess tasks: spawn fails with `ENOSYS`. Authorization and streams
     work there.
   - Tests that spawn processes must run natively.
7. **Orphans need the cgroup.** `solti-exec` puts each task in its own process group and
   sets no parent-death signal. If the daemon is killed outright, a task that writes
   nothing survives it. Under systemd, `KillMode=control-group` (the default) cleans
   up, so `lmx.service` and the transient `systemd-run` unit must keep it.
8. **Every task needs an explicit `PATH` on NixOS.** `solti-exec` clears the
   environment, and NixOS's shell has no default `PATH`. In the guest, the task's
   `/bin/sh` could not find `sleep`.
   - `lmxd` must give each task a `PATH`, or call tools by store path.
   - Examples of such tools: `nix-store`, `nixos-rebuild`, `systemctl`.
   - The design's "explicit `PATH`, cleared environment" is mandatory, not a hardening
     option.
9. **Core dumps.** A watchdog abort sends `SIGABRT` to every process in the unit, and
   `systemd-coredump` takes each one. `LimitCORE` decides whether cores are stored.
   `lmx.service` should set it explicitly.

## SDK changes

Made in the SDK working tree, not committed:

| File | Change |
|---|---|
| `crates/solti-exec/src/host/limits.rs` | `RlimitResource` is `__rlimit_resource_t` only for glibc and uClibc on Linux; musl, Android and others use `c_int`. Android had the same bug. |
| `crates/solti-exec/src/host/cgroups.rs` | `f_type` is unsigned on musl; the cast to `u64` keeps `#[allow(clippy::unnecessary_cast)]`, as elsewhere in the crate. |
| `crates/solti-exec/src/isolation/seccomp.rs` | `libc` has no `SYS_kexec_file_load` for aarch64 musl; the denylist uses the asm-generic number 294 there. |

How they were checked:
- `solti-exec` with all features: fmt; clippy with `-D warnings` for x86_64 and aarch64
  musl and aarch64 glibc; tests on aarch64 musl and glibc (344 pass on each);
  clippy on macOS.
- `solti-model`, `solti-runner`, `solti-core` and `solti-observe` with all features:
  clippy for both musl targets.

Proposed, not made:
- **Document identity from request extensions.** Document that `solti-api` takes
  `ApiIdentity` from the request extensions when no authenticator is set. A
  `PeerCredentials` interceptor could follow under the `grpc` feature. `lmxd` does not
  need it: the spike's interceptor is 20 lines.
- **A journald field prefix option** in `solti-observe`, if `LMX_TASK` must keep its
  name (finding 4).
- **A Rust way to build Task API requests from domain types.** Today `grpc::convert` is
  private. This matters only if `lmx` creates tasks through the Task API rather than
  the `lmx` service; M2 decides.
- **No systemd helpers needed.** `listenfd` and `sd-notify` cover socket activation and
  notify in about 30 lines.
