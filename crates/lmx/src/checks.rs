//! Checks of `lmx doctor` and `lmx net check`: their names and what each one checks.

use lmx_model::{Check, CheckStatus};

/// A check that `lmx doctor` or `lmx net check` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckId {
    /// The configuration of the generation.
    Config,
    /// `lmxd`.
    Owner,
    /// The desired, built and booted generations.
    Generations,
    /// Free space on the guest disk.
    Disk,
    /// The links of the dotfiles.
    Dotfiles,
    /// The guest firewall.
    Firewall,
    /// What listens on the port.
    Listener,
    /// The process behind the socket.
    Process,
}

/// The checks of `lmx doctor`, in the order it reports them.
pub(crate) const DOCTOR: [CheckId; 5] = [
    CheckId::Config,
    CheckId::Owner,
    CheckId::Generations,
    CheckId::Disk,
    CheckId::Dotfiles,
];

/// The checks of `lmx net check`, in the order it reports them.
pub(crate) const NET: [CheckId; 3] = [CheckId::Firewall, CheckId::Listener, CheckId::Process];

impl CheckId {
    /// Name of the check in the output, such as `config`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Owner => "owner",
            Self::Generations => "generations",
            Self::Disk => "disk",
            Self::Dotfiles => "dotfiles",
            Self::Firewall => "firewall",
            Self::Listener => "listener",
            Self::Process => "process",
        }
    }

    /// What the check checks.
    #[cfg(test)]
    pub(crate) const fn what(self) -> &'static str {
        match self {
            Self::Config => "`/etc/lmx/config.json` is readable and valid",
            Self::Owner => "`lmxd` answers, with the same version as `lmx`",
            Self::Generations => {
                "The state of the generations, from `lmxd` or, without it, from the generation \
                 markers"
            }
            Self::Disk => "Shown when `lmxd` reports that the disk is low",
            Self::Dotfiles => {
                "Shown when `limanix.toml` names dotfiles: their entries are linked into the home. \
                 A `warning` names the paths in the way; `failed` means the directory is not \
                 mounted or cannot be read"
            }
            Self::Firewall => {
                "The guest firewall opens the port for its protocol. TCP port 22 is always open \
                 for SSH"
            }
            Self::Listener => "Something listens on the port, and not only on a loopback address",
            Self::Process => {
                "Which process holds the socket, and which user created it. Only when something \
                 listens. Docker's proxy is a warning: `network.ports` does not control the ports \
                 Docker publishes"
            }
        }
    }

    /// A finding of this check.
    pub(crate) fn finding(self, status: CheckStatus, message: impl Into<String>) -> Check {
        Check::new(self.name(), status, message)
    }
}
