//! `lmxd`: the guest owner daemon of a LimaNix VM.
//!
//! systemd starts it as root through `lmx.socket`, which passes the listening socket, so the socket
//! exists while the daemon restarts. Started without one, as the transient daemon of an update,
//! it binds the socket itself. It reports readiness and feeds the watchdog when systemd asks for
//! them, and stops in order on SIGTERM or SIGINT.
#![forbid(unsafe_code)]

use std::{
    fs::{self, Permissions},
    io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use clap::Parser;
use lmx_facts::disk::{self, STORE_PATH};
use lmx_ipc::SOCKET_PATH;
use lmx_model::{CONFIG_PATH, Config};
use lmxd::{Daemon, GuardSchedule, Options};
use sd_notify::NotifyState;
use solti::observe::{LoggerConfig, LoggerFormat, init_logger};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
    time::MissedTickBehavior,
};

/// Guest owner daemon of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmxd", version)]
struct Args {
    /// Platform configuration.
    #[arg(long, default_value = CONFIG_PATH)]
    config: PathBuf,
    /// Socket to bind when systemd passes none.
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    logging();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("lmxd: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("lmxd: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Starts the daemon and serves until a stop signal.
async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load(&args.config)?;
    let listener = listener(&args.socket)?;
    let daemon = Daemon::start(Options {
        config,
        usage: Arc::new(|| disk::usage(Path::new(STORE_PATH)).map_err(|error| error.to_string())),
        guard: Some(GuardSchedule::after_boot(uptime())),
    })
    .await?;
    let _ = sd_notify::notify(false, &[NotifyState::Ready]);
    feed_watchdog();
    tracing::info!("lmxd {} is ready", env!("CARGO_PKG_VERSION"));
    daemon.serve(listener, stop_signal()).await?;
    tracing::info!("lmxd stopped");
    Ok(())
}

/// Logs to journald under systemd and to standard error otherwise.
fn logging() {
    let format = if std::env::var_os("JOURNAL_STREAM").is_some() {
        LoggerFormat::Journald
    } else {
        LoggerFormat::Text
    };
    if let Err(error) = init_logger(&LoggerConfig {
        format,
        ..LoggerConfig::default()
    }) {
        eprintln!("lmxd: logging is unavailable: {error}");
    }
}

/// The socket systemd passed, or a socket bound at `path` that every user may connect to.
fn listener(path: &Path) -> io::Result<UnixListener> {
    if let Some(listener) = listenfd::ListenFd::from_env().take_unix_listener(0)? {
        listener.set_nonblocking(true)?;
        return UnixListener::from_std(listener);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A socket left by an earlier daemon is replaced; anything else at the path is not ours.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    // Every user may connect; peer credentials decide what each caller may do.
    fs::set_permissions(path, Permissions::from_mode(0o666))?;
    Ok(listener)
}

/// How long the system has been up; unknown counts as long, so the guard checks at once.
fn uptime() -> Duration {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .map_or(Duration::MAX, Duration::from_secs_f64)
}

/// Feeds the systemd watchdog at half its interval, when systemd set one.
fn feed_watchdog() {
    let mut usec = 0;
    if !sd_notify::watchdog_enabled(false, &mut usec) {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_micros(usec / 2));
        // After a stall, one ping is enough; a burst would hide how long the stall was.
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let _ = sd_notify::notify(false, &[NotifyState::Watchdog]);
        }
    });
}

/// Completes on SIGTERM or SIGINT, after telling systemd that the daemon is stopping.
async fn stop_signal() {
    let (Ok(mut terminate), Ok(mut interrupt)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        tracing::error!("stop signals cannot be handled; waiting for SIGKILL");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    let _ = sd_notify::notify(false, &[NotifyState::Stopping]);
    tracing::info!("stopping");
}
