//! `lmx version`: the binary and the host contract it speaks.

use std::{io, process::ExitCode};

use lmx_model::{CONTRACT_VERSION, Envelope};
use serde::Serialize;

use crate::{cli::OutputArgs, output};

/// Version answer of the host contract.
#[derive(Debug, Serialize)]
pub(crate) struct Version {
    /// Release version of the binary.
    version: &'static str,
    /// Host contract version.
    contract: u32,
}

/// Runs `lmx version`.
pub(crate) fn run(args: &OutputArgs) -> io::Result<ExitCode> {
    let version = Version {
        version: env!("CARGO_PKG_VERSION"),
        contract: CONTRACT_VERSION,
    };
    if args.json {
        output::write_json(&Envelope::success(version))?;
    } else {
        output::write_text(&format!(
            "lmx {} (host contract {})\n",
            version.version, version.contract
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}
