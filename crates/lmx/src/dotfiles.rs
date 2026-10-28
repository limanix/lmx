//! `lmx dotfiles`: the dotfiles directory from the Mac.
//!
//! `[shell] dotfiles` of `limanix.toml` names a directory on the Mac that is shaped like a home.
//! The host mounts it into the guest and renders its guest path into
//! [`Config::dotfiles`](lmx_model::Config::dotfiles). `lmx dotfiles link` links its entries into
//! the home of the configuration, never into `$HOME`:
//!
//! | In the dotfiles                                     | In the home                      |
//! |-----------------------------------------------------|----------------------------------|
//! | a top-level entry whose name starts with `.`        | a link of the same name          |
//! | an entry of `.config`, such as `.config/nvim`       | a link in `~/.config`            |
//! | a [`SKIPPED`] name, or a name without a leading `.` | nothing                          |
//!
//! `~/.config` itself stays a directory of the guest, created when missing, where tools keep their
//! own files beside the links. A path the home already has decides what happens to it:
//!
//! | The path in the home                                 | `lmx dotfiles link`            |
//! |------------------------------------------------------|--------------------------------|
//! | is missing                                           | links it to its entry          |
//! | links to its entry                                   | leaves it                      |
//! | links elsewhere inside the dotfiles                  | points it to its entry         |
//! | links outside the dotfiles, or is a file or a folder | leaves it, as a conflict       |
//!
//! A link in the home or in `~/.config` to the same name in the dotfiles is removed when that name
//! is gone; nothing else is ever removed. While the dotfiles are not a shared folder of the mount
//! table, nothing is linked or removed: the empty mount point that a failed mount leaves would
//! make every link look stale. A change on the Mac is live in the guest at once: `lmx` manages only
//! the links. Every interactive shell of the account runs `lmx dotfiles link --quiet`
//! before it reads the account's startup files, `~/.zshenv` and `~/.bash_profile` included, so a
//! run reads four directories and changes only what differs. Shells that start together, such as the panes of a tmux session, may run it at the same
//! time: a change another run made first is no failure.
//!
//! Reading, planning and changing are separate steps: [`plan`] is a pure function of what [`read`]
//! found, and its tests feed it listings.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, DirEntry},
    io,
    os::unix::fs::symlink,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use lmx_facts::{FactError, mounts};
use lmx_model::{Check, CheckStatus, Config};

use crate::{
    output,
    palette::{Color, Paint, Palette},
    system::System,
    welcome::listing,
};

/// Names that are never linked: repository files, Finder's metadata, and `.config`, whose entries
/// are linked one by one instead. Inside `.config`, the same names are skipped.
const SKIPPED: [&str; 6] = [
    ".git",
    ".github",
    ".gitignore",
    ".gitmodules",
    ".DS_Store",
    ".config",
];

/// The directory whose entries are linked one by one, in the dotfiles and in the home.
const CONFIG: &str = ".config";

/// Columns of the word that starts a line, such as `linked`.
const WORD: usize = 10;

/// What `lmx dotfiles` says in a VM without dotfiles.
const UNCONFIGURED: &str = "\
No dotfiles are configured in this VM.
Set [shell] dotfiles in limanix.toml, then run limanix update on the Mac.
";

/// What a path in the home is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// A symbolic link, with the path it holds.
    Link(PathBuf),
    /// A directory.
    Directory,
    /// A regular file, or anything else that is neither a link nor a directory.
    File,
}

/// The dotfiles and the home, read once for a command.
#[derive(Debug, Default)]
struct Listing {
    /// Names in the dotfiles directory.
    entries: BTreeSet<OsString>,
    /// Names in its `.config`; none when it has no `.config` directory.
    config_entries: BTreeSet<OsString>,
    /// What each entry of the home is.
    home: BTreeMap<OsString, Kind>,
    /// What each entry of `~/.config` is, when `~/.config` is a directory.
    config_home: BTreeMap<OsString, Kind>,
}

/// A change to the home. Paths in the home are relative to it; entries of the dotfiles are
/// absolute.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Change {
    /// Create `~/.config`, which holds the links to the entries of `.config`.
    ConfigDirectory,
    /// Link the missing `target` to `entry`.
    Link {
        /// Path in the home.
        target: PathBuf,
        /// Entry of the dotfiles.
        entry: PathBuf,
    },
    /// Point `target`, a link to `from` elsewhere inside the dotfiles, to `entry`.
    Repoint {
        /// Path in the home.
        target: PathBuf,
        /// Entry of the dotfiles.
        entry: PathBuf,
        /// Where the link points now.
        from: PathBuf,
    },
    /// Remove `target`, a link to `gone`, a path inside the dotfiles that no longer exists.
    Remove {
        /// Path in the home.
        target: PathBuf,
        /// Where the link points.
        gone: PathBuf,
    },
}

impl Change {
    /// The path in the home that the change makes or removes.
    fn target(&self) -> &Path {
        match self {
            Self::ConfigDirectory => Path::new(CONFIG),
            Self::Link { target, .. }
            | Self::Repoint { target, .. }
            | Self::Remove { target, .. } => target,
        }
    }
}

/// A path in the home that is in the way of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Conflict {
    /// The path, relative to the home.
    target: PathBuf,
    /// What it is.
    found: Kind,
}

/// The state of the home and the changes that link the dotfiles into it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    /// Paths in the home that already link to their entry.
    linked: Vec<PathBuf>,
    /// Changes in the order they are made: `~/.config` before the links in it.
    changes: Vec<Change>,
    /// Paths in the home in the way of an entry; nothing changes them.
    conflicts: Vec<Conflict>,
}

/// Why the dotfiles cannot be compared with the home.
#[derive(Debug)]
enum ReadError {
    /// The configuration names a path that is not absolute; the platform renders it.
    NotAbsolute(PathBuf),
    /// The mount table cannot be read, so whether the host mounted the dotfiles is unknown.
    MountTable(FactError),
    /// The mount table has no shared folder at the dotfiles: the host has not mounted them.
    NotMounted(PathBuf),
    /// A directory of the dotfiles, which the host mounts, cannot be read.
    Dotfiles {
        /// The directory.
        path: PathBuf,
        /// The failure.
        source: io::Error,
    },
    /// A directory of the home cannot be read.
    Home {
        /// The directory.
        path: PathBuf,
        /// The failure.
        source: io::Error,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute(path) => write!(
                formatter,
                "the configuration names {}, which is not an absolute path",
                path.display()
            ),
            Self::MountTable(error) => write!(formatter, "{error}"),
            Self::NotMounted(path) => {
                write!(
                    formatter,
                    "the dotfiles at {} are not mounted",
                    path.display()
                )
            }
            Self::Dotfiles { path, source } => write!(
                formatter,
                "cannot read the dotfiles at {}: {source}",
                path.display()
            ),
            Self::Home { path, source } => {
                write!(formatter, "cannot read {}: {source}", path.display())
            }
        }
    }
}

/// Runs `lmx dotfiles`: shows the linked entries, the conflicts and the missing links, and changes
/// nothing.
pub(crate) fn status(system: &System) -> io::Result<ExitCode> {
    let Some(config) = config(system) else {
        return Ok(ExitCode::from(output::FAILURE));
    };
    let Some(dotfiles) = config.dotfiles.as_deref().map(Path::new) else {
        output::write_text(UNCONFIGURED)?;
        return Ok(ExitCode::SUCCESS);
    };
    let home = Path::new(&config.user.home);
    match survey(dotfiles, home, &system.mountinfo()) {
        Ok(plan) => {
            let paint = Paint::detect(Palette::of_config(&system.config));
            output::write_text(&render(dotfiles, home, &plan, paint))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("lmx: {error}");
            Ok(ExitCode::from(output::FAILURE))
        }
    }
}

/// Runs `lmx dotfiles link`: links the dotfiles into the home and prints each change and conflict,
/// or with `quiet` only errors.
///
/// It fails when a directory cannot be read or a change fails, and never as another user than the
/// development account: `sudo` would leave links owned by root in its home. A conflict does not
/// fail it, and without dotfiles it does nothing.
pub(crate) fn link(system: &System, quiet: bool) -> io::Result<ExitCode> {
    let Some(config) = config(system) else {
        return Ok(ExitCode::from(output::FAILURE));
    };
    let Some(dotfiles) = config.dotfiles.as_deref().map(Path::new) else {
        if !quiet {
            output::write_text(UNCONFIGURED)?;
        }
        return Ok(ExitCode::SUCCESS);
    };
    let user = &config.user;
    let uid = rustix::process::geteuid().as_raw();
    if uid != user.uid {
        eprintln!(
            "lmx: lmx dotfiles link runs only as {} (uid {}), not as uid {uid}: the links in {} \
             belong to {}",
            user.name, user.uid, user.home, user.name
        );
        return Ok(ExitCode::from(output::FAILURE));
    }
    let home = Path::new(&user.home);
    let plan = match survey(dotfiles, home, &system.mountinfo()) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("lmx: {error}");
            return Ok(ExitCode::from(output::FAILURE));
        }
    };
    let paint = Paint::detect(Palette::of_config(&system.config));
    let mut text = String::new();
    let mut failed = false;
    let mut blocked = false;

    for change in &plan.changes {
        if blocked && change.target().starts_with(CONFIG) {
            continue;
        }
        match apply(home, change) {
            Ok(true) => text.push_str(&made(change, paint)),
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "lmx: cannot {} {}: {error}",
                    verb(change),
                    shown(change.target())
                );
                failed = true;
                blocked |= *change == Change::ConfigDirectory;
            }
        }
    }
    for conflict in &plan.conflicts {
        text.push_str(&conflicting(conflict, paint));
    }
    if !quiet {
        output::write_text(&text)?;
    }
    Ok(output::page_status(!failed))
}

/// The `dotfiles` check of `lmx doctor`, when the configuration names dotfiles.
pub(crate) fn check(system: &System) -> Option<Check> {
    let config = system.config.as_ref().ok()?;
    let dotfiles = Path::new(config.dotfiles.as_deref()?);
    Some(finding(
        dotfiles,
        &survey(dotfiles, Path::new(&config.user.home), &system.mountinfo()),
    ))
}

/// The configuration, or `None` after saying why it cannot be read.
fn config(system: &System) -> Option<&Config> {
    system
        .config
        .as_ref()
        .map_err(|message| eprintln!("lmx: {message}"))
        .ok()
}

/// Reads the dotfiles at `dotfiles` and the home at `home`, and plans the links between them, once
/// the mount table at `mountinfo` shows that the host mounted the dotfiles.
fn survey(dotfiles: &Path, home: &Path, mountinfo: &Path) -> Result<Plan, ReadError> {
    if let Some(path) = [dotfiles, home]
        .into_iter()
        .find(|path| !path.is_absolute())
    {
        return Err(ReadError::NotAbsolute(path.to_path_buf()));
    }
    mounted(dotfiles, mountinfo)?;
    Ok(plan(dotfiles, &read(dotfiles, home)?))
}

/// Whether the mount table at `mountinfo` has a shared folder, virtiofs or 9p, at `dotfiles`.
fn mounted(dotfiles: &Path, mountinfo: &Path) -> Result<(), ReadError> {
    let shared = mounts::shared(mountinfo).map_err(ReadError::MountTable)?;
    if shared
        .iter()
        .any(|mount| Path::new(&mount.target) == dotfiles)
    {
        Ok(())
    } else {
        Err(ReadError::NotMounted(dotfiles.to_path_buf()))
    }
}

/// Reads the names in the dotfiles and in their `.config`, and what the entries of the home and of
/// `~/.config` are.
fn read(dotfiles: &Path, home: &Path) -> Result<Listing, ReadError> {
    let entries = names(dotfiles).map_err(|source| ReadError::Dotfiles {
        path: dotfiles.to_path_buf(),
        source,
    })?;
    let config_entries = if entries.contains(OsStr::new(CONFIG)) {
        let path = dotfiles.join(CONFIG);
        match names(&path) {
            Ok(names) => names,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                BTreeSet::new()
            }
            Err(source) => return Err(ReadError::Dotfiles { path, source }),
        }
    } else {
        BTreeSet::new()
    };
    let home_entries = kinds(home).map_err(|source| ReadError::Home {
        path: home.to_path_buf(),
        source,
    })?;
    let config_home = if home_entries.get(OsStr::new(CONFIG)) == Some(&Kind::Directory) {
        let path = home.join(CONFIG);
        kinds(&path).map_err(|source| ReadError::Home { path, source })?
    } else {
        BTreeMap::new()
    };
    Ok(Listing {
        entries,
        config_entries,
        home: home_entries,
        config_home,
    })
}

/// The names of the entries of the directory at `path`.
fn names(path: &Path) -> io::Result<BTreeSet<OsString>> {
    fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}

/// What each entry of the directory at `path` is; an entry removed while it is read is left out.
fn kinds(path: &Path) -> io::Result<BTreeMap<OsString, Kind>> {
    let mut kinds = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        match kind(&entry) {
            Ok(kind) => {
                kinds.insert(entry.file_name(), kind);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(kinds)
}

/// What a directory entry is; the type comes with the listing, and only a link needs another call.
fn kind(entry: &DirEntry) -> io::Result<Kind> {
    let file_type = entry.file_type()?;
    Ok(if file_type.is_symlink() {
        Kind::Link(fs::read_link(entry.path())?)
    } else if file_type.is_dir() {
        Kind::Directory
    } else {
        Kind::File
    })
}

/// Plans the changes that link the entries in `listing` from the dotfiles at `dotfiles`, an
/// absolute path, into the home.
fn plan(dotfiles: &Path, listing: &Listing) -> Plan {
    let mut plan = Plan::default();
    let top: Vec<&OsStr> = listing
        .entries
        .iter()
        .map(OsString::as_os_str)
        .filter(|name| name.as_encoded_bytes().starts_with(b".") && !skipped(name))
        .collect();
    let config: Vec<&OsStr> = listing
        .config_entries
        .iter()
        .map(OsString::as_os_str)
        .filter(|name| !skipped(name))
        .collect();

    for name in &top {
        plan.target(dotfiles, PathBuf::from(name), listing.home.get(*name));
    }
    let config_home = listing.home.get(OsStr::new(CONFIG));
    if !config.is_empty() {
        match config_home {
            None | Some(Kind::Directory) => {
                if config_home.is_none() {
                    plan.changes.push(Change::ConfigDirectory);
                }
                for name in &config {
                    let target = Path::new(CONFIG).join(name);
                    plan.target(dotfiles, target, listing.config_home.get(*name));
                }
            }
            Some(found) => plan.conflicts.push(Conflict {
                target: PathBuf::from(CONFIG),
                found: found.clone(),
            }),
        }
    }

    for (name, found) in &listing.home {
        let planned = top.contains(&name.as_os_str()) || (name == CONFIG && !config.is_empty());
        if !planned {
            plan.stale(dotfiles, listing, PathBuf::from(name), found);
        }
    }
    if config_home == Some(&Kind::Directory) {
        for (name, found) in &listing.config_home {
            if !config.contains(&name.as_os_str()) {
                plan.stale(dotfiles, listing, Path::new(CONFIG).join(name), found);
            }
        }
    }
    plan
}

impl Plan {
    /// Adds what `target` needs to link to its entry, given what the home has there.
    fn target(&mut self, dotfiles: &Path, target: PathBuf, found: Option<&Kind>) {
        let entry = dotfiles.join(&target);
        match found {
            None => self.changes.push(Change::Link { target, entry }),
            Some(Kind::Link(to)) if *to == entry => self.linked.push(target),
            Some(Kind::Link(to)) if inside(to, dotfiles).is_some() => {
                self.changes.push(Change::Repoint {
                    target,
                    entry,
                    from: to.clone(),
                });
            }
            Some(found) => self.conflicts.push(Conflict {
                target,
                found: found.clone(),
            }),
        }
    }

    /// Adds the removal of `target` when it links to the same name in the dotfiles and that name
    /// is gone.
    fn stale(&mut self, dotfiles: &Path, listing: &Listing, target: PathBuf, found: &Kind) {
        let Kind::Link(to) = found else {
            return;
        };
        let names = if target.parent() == Some(Path::new(CONFIG)) {
            &listing.config_entries
        } else {
            &listing.entries
        };
        let own = inside(to, dotfiles) == Some(target.as_path());
        if own && target.file_name().is_some_and(|name| !names.contains(name)) {
            self.changes.push(Change::Remove {
                target,
                gone: to.clone(),
            });
        }
    }
}

/// Whether `name` is never linked.
fn skipped(name: &OsStr) -> bool {
    SKIPPED.iter().any(|skipped| name == OsStr::new(skipped))
}

/// The part of `path` below `dotfiles`, when `path` is absolute, inside `dotfiles` and free of
/// `..`; a relative link is never one of `lmx`'s.
fn inside<'a>(path: &'a Path, dotfiles: &Path) -> Option<&'a Path> {
    let inner = path.strip_prefix(dotfiles).ok()?;
    inner
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(inner)
}

/// Makes `change` in `home`: `Ok(false)` when another run made it first, or when the path changed
/// after it was read and stays as it is now.
fn apply(home: &Path, change: &Change) -> io::Result<bool> {
    let path = home.join(change.target());
    match change {
        Change::ConfigDirectory => match fs::create_dir(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {
                Ok(false)
            }
            Err(error) => Err(error),
        },
        Change::Link { entry, .. } => create_link(&path, entry),
        Change::Repoint { entry, from, .. } => {
            if !links_to(&path, from) {
                return Ok(false);
            }
            remove_link(&path)?;
            create_link(&path, entry)
        }
        Change::Remove { gone, .. } => {
            if !links_to(&path, gone) {
                return Ok(false);
            }
            remove_link(&path)
        }
    }
}

/// Whether `path` is still a link to `to`.
fn links_to(path: &Path, to: &Path) -> bool {
    fs::read_link(path).is_ok_and(|target| target.as_path() == to)
}

/// Links `path` to `entry`; the same link made first by another run is no failure.
fn create_link(path: &Path, entry: &Path) -> io::Result<bool> {
    match symlink(entry, path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && links_to(path, entry) => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Removes the link `path`; a link another run removed first is no failure.
fn remove_link(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// What a failed change was about to do, for its error: `cannot link ~/.zshrc`.
const fn verb(change: &Change) -> &'static str {
    match change {
        Change::ConfigDirectory => "create",
        Change::Link { .. } | Change::Repoint { .. } => "link",
        Change::Remove { .. } => "remove",
    }
}

/// The line of a change `lmx dotfiles link` made; creating `~/.config` has none.
fn made(change: &Change, paint: Paint) -> String {
    let palette = paint.palette();
    match change {
        Change::ConfigDirectory => String::new(),
        Change::Link { target, .. } => line(paint, palette.green, "linked", target, ""),
        Change::Repoint { target, .. } => line(paint, palette.green, "repointed", target, ""),
        Change::Remove { target, .. } => line(paint, palette.muted, "removed", target, ""),
    }
}

/// The line of a conflict: the path, and what it is.
fn conflicting(conflict: &Conflict, paint: Paint) -> String {
    line(
        paint,
        paint.palette().yellow,
        "conflict",
        &conflict.target,
        &found(&conflict.found),
    )
}

/// The page of `lmx dotfiles`: where the dotfiles come from, each path and its state, then what to
/// do about the rest.
fn render(dotfiles: &Path, home: &Path, plan: &Plan, paint: Paint) -> String {
    let palette = paint.palette();
    let label = |name: &str| paint.color(palette.muted, &format!("{name:<WORD$}"));
    let mut text = format!(
        "{}{}\n{}{}\n\n",
        label("Dotfiles"),
        dotfiles.display(),
        label("Home"),
        home.display()
    );
    for target in &plan.linked {
        text.push_str(&line(paint, palette.green, "linked", target, ""));
    }
    let mut pending = false;
    for change in &plan.changes {
        let (word, color, target, detail) = match change {
            Change::ConfigDirectory => continue,
            Change::Link { target, .. } => ("missing", palette.blue, target, String::new()),
            Change::Repoint { target, from, .. } => {
                ("missing", palette.blue, target, points_to(from))
            }
            Change::Remove { target, gone } => ("stale", palette.muted, target, points_to(gone)),
        };
        text.push_str(&line(paint, color, word, target, &detail));
        pending = true;
    }
    for conflict in &plan.conflicts {
        text.push_str(&conflicting(conflict, paint));
    }
    if plan.linked.is_empty() && !pending && plan.conflicts.is_empty() {
        text.push_str("The dotfiles have nothing to link.\n");
    }
    if pending || !plan.conflicts.is_empty() {
        text.push('\n');
    }
    if pending {
        text.push_str("Open a new shell, or run lmx dotfiles link, to update the links.\n");
    }
    if !plan.conflicts.is_empty() {
        text.push_str("Move the conflicting paths aside, then open a new shell.\n");
    }
    text
}

/// The finding of the `dotfiles` check, from what comparing the dotfiles with the home gave.
fn finding(dotfiles: &Path, survey: &Result<Plan, ReadError>) -> Check {
    let plan = match survey {
        Ok(plan) => plan,
        Err(error) => {
            let check = Check::new("dotfiles", CheckStatus::Failed, error.to_string());
            return match error {
                ReadError::NotAbsolute(_) => {
                    check.hint("The platform renders it; limanix update on the Mac restores it.")
                }
                ReadError::Dotfiles { .. } | ReadError::NotMounted(_) => check.hint(
                    "The host mounts the directory; limanix update on the Mac mounts it again.",
                ),
                ReadError::MountTable(_) | ReadError::Home { .. } => check,
            };
        }
    };
    let dotfiles = dotfiles.display();
    if !plan.conflicts.is_empty() {
        let paths: Vec<String> = plan
            .conflicts
            .iter()
            .map(|conflict| shown(&conflict.target))
            .collect();
        let (verb, them) = if paths.len() == 1 {
            ("is", "it")
        } else {
            ("are", "them")
        };
        return Check::new(
            "dotfiles",
            CheckStatus::Warning,
            format!(
                "{} {verb} in the way of the dotfiles from {dotfiles}.",
                listing(&paths)
            ),
        )
        .hint(format!("Move {them} aside, then open a new shell."));
    }
    let linked = plan.linked.len();
    let missing = plan
        .changes
        .iter()
        .filter(|change| matches!(change, Change::Link { .. } | Change::Repoint { .. }))
        .count();
    let counted = match linked {
        1 => format!("1 dotfile from {dotfiles} is linked"),
        _ => format!("{linked} dotfiles from {dotfiles} are linked"),
    };
    let message = match (linked, missing) {
        (0, 0) => format!("The dotfiles at {dotfiles} have nothing to link."),
        (_, 0) => format!("{counted}."),
        _ => format!("{counted}; the next shell links {missing} more."),
    };
    Check::new("dotfiles", CheckStatus::Ok, message)
}

/// A line of a path in the home: the word in `color`, the path, then `detail`.
fn line(paint: Paint, color: Color, word: &str, target: &Path, detail: &str) -> String {
    format!(
        "{}{}{detail}\n",
        paint.color(color, &format!("{word:<WORD$}")),
        shown(target)
    )
}

/// What a path in the way is, after the path: ` is a file`.
fn found(kind: &Kind) -> String {
    match kind {
        Kind::Link(to) => points_to(to),
        Kind::Directory => " is a directory".to_owned(),
        Kind::File => " is a file".to_owned(),
    }
}

/// Where a link points, after its path: ` links to /mnt/dotfiles/.vimrc`.
fn points_to(to: &Path) -> String {
    format!(" links to {}", to.display())
}

/// A path in the home as people write it, such as `~/.zshrc`.
fn shown(target: &Path) -> String {
    format!("~/{}", target.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOTFILES: &str = "/mnt/dotfiles";

    fn set(names: &[&str]) -> BTreeSet<OsString> {
        names.iter().map(OsString::from).collect()
    }

    fn entries(entries: &[(&str, Kind)]) -> BTreeMap<OsString, Kind> {
        entries
            .iter()
            .map(|(name, kind)| (OsString::from(name), kind.clone()))
            .collect()
    }

    fn link_to(path: &str) -> Kind {
        Kind::Link(PathBuf::from(path))
    }

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn linking(target: &str) -> Change {
        Change::Link {
            target: path(target),
            entry: Path::new(DOTFILES).join(target),
        }
    }

    fn listing() -> Listing {
        Listing {
            entries: set(&[
                ".zshrc",
                ".tmux.conf",
                ".gitconfig",
                ".config",
                ".git",
                ".github",
                ".gitignore",
                ".gitmodules",
                ".DS_Store",
                "README.md",
                "install.sh",
            ]),
            config_entries: set(&["nvim", "starship.toml", ".DS_Store"]),
            ..Listing::default()
        }
    }

    fn removed(plan: &Plan) -> Vec<&Path> {
        plan.changes
            .iter()
            .filter_map(|change| match change {
                Change::Remove { target, .. } => Some(target.as_path()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn links_the_dot_entries_and_the_entries_of_config_one_by_one() {
        assert_eq!(
            plan(Path::new(DOTFILES), &listing()),
            Plan {
                linked: vec![],
                changes: vec![
                    linking(".gitconfig"),
                    linking(".tmux.conf"),
                    linking(".zshrc"),
                    Change::ConfigDirectory,
                    linking(".config/nvim"),
                    linking(".config/starship.toml"),
                ],
                conflicts: vec![],
            }
        );
    }

    #[test]
    fn keeps_its_links_and_repoints_links_elsewhere_in_the_dotfiles() {
        let mut listing = listing();
        listing.home = entries(&[
            (".zshrc", link_to("/mnt/dotfiles/.zshrc")),
            (".tmux.conf", link_to("/mnt/dotfiles/tmux/tmux.conf")),
            (".gitconfig", link_to("/mnt/dotfiles//.gitconfig")),
            (".config", Kind::Directory),
        ]);
        listing.config_home = entries(&[
            ("nvim", link_to("/mnt/dotfiles/.config/nvim/")),
            ("starship.toml", link_to("/mnt/dotfiles")),
        ]);
        let plan = plan(Path::new(DOTFILES), &listing);
        assert_eq!(
            plan.linked,
            [path(".gitconfig"), path(".zshrc"), path(".config/nvim")]
        );
        assert_eq!(
            plan.changes,
            [
                Change::Repoint {
                    target: path(".tmux.conf"),
                    entry: path("/mnt/dotfiles/.tmux.conf"),
                    from: path("/mnt/dotfiles/tmux/tmux.conf"),
                },
                Change::Repoint {
                    target: path(".config/starship.toml"),
                    entry: path("/mnt/dotfiles/.config/starship.toml"),
                    from: path("/mnt/dotfiles"),
                },
            ]
        );
        assert!(plan.conflicts.is_empty(), "{plan:?}");
    }

    #[test]
    fn leaves_what_is_not_its_link_in_place_as_a_conflict() {
        let mut listing = listing();
        listing.home = entries(&[
            (".zshrc", Kind::File),
            (".tmux.conf", link_to("/home/dev/tmux.conf")),
            (".gitconfig", link_to("/mnt/dotfiles/../etc/gitconfig")),
            (".config", Kind::Directory),
        ]);
        listing.config_home = entries(&[
            ("nvim", Kind::Directory),
            (
                "starship.toml",
                link_to("../../mnt/dotfiles/.config/starship.toml"),
            ),
        ]);
        let plan = plan(Path::new(DOTFILES), &listing);
        assert!(
            plan.linked.is_empty() && plan.changes.is_empty(),
            "{plan:?}"
        );
        let conflict = |target: &str, found: Kind| Conflict {
            target: path(target),
            found,
        };
        assert_eq!(
            plan.conflicts,
            [
                conflict(".gitconfig", link_to("/mnt/dotfiles/../etc/gitconfig")),
                conflict(".tmux.conf", link_to("/home/dev/tmux.conf")),
                conflict(".zshrc", Kind::File),
                conflict(".config/nvim", Kind::Directory),
                conflict(
                    ".config/starship.toml",
                    link_to("../../mnt/dotfiles/.config/starship.toml")
                ),
            ]
        );
    }

    #[test]
    fn removes_only_its_links_to_paths_that_are_gone() {
        let mut listing = listing();
        listing.home = entries(&[
            (".vimrc", link_to("/mnt/dotfiles/.vimrc")),
            (".profile", link_to("/mnt/dotfiles/shell/profile")),
            (".gitignore", link_to("/mnt/dotfiles/.gitignore")),
            ("notes", link_to("/mnt/dotfiles/README.md")),
            (".inputrc", link_to("/etc/inputrc")),
            (".escape", link_to("/mnt/dotfiles/../gone")),
            (".relative", link_to("../../mnt/dotfiles/.gone")),
            (".bashrc", Kind::File),
            (".config", Kind::Directory),
        ]);
        listing.config_home = entries(&[
            ("git", link_to("/mnt/dotfiles/.config/git")),
            ("htop", link_to("/mnt/dotfiles/.config/nvim/htop")),
            ("fish", Kind::Directory),
        ]);
        let plan = plan(Path::new(DOTFILES), &listing);
        assert_eq!(
            removed(&plan),
            [Path::new(".vimrc"), Path::new(".config/git")]
        );
        assert!(plan.conflicts.is_empty(), "{plan:?}");
    }

    #[test]
    fn keeps_config_a_directory_of_the_guest() {
        for found in [link_to("/mnt/dotfiles/.config"), Kind::File] {
            let mut listing = listing();
            listing.home = entries(&[(".config", found.clone())]);
            let plan = plan(Path::new(DOTFILES), &listing);
            assert_eq!(
                plan.conflicts,
                [Conflict {
                    target: path(".config"),
                    found
                }]
            );
            assert_eq!(
                plan.changes,
                [
                    linking(".gitconfig"),
                    linking(".tmux.conf"),
                    linking(".zshrc")
                ]
            );
        }

        let mut listing = listing();
        listing.config_entries = set(&[".DS_Store"]);
        assert!(
            !plan(Path::new(DOTFILES), &listing)
                .changes
                .contains(&Change::ConfigDirectory)
        );
        listing.home = entries(&[(".config", link_to("/home/dev/settings"))]);
        assert!(plan(Path::new(DOTFILES), &listing).conflicts.is_empty());

        listing.entries.remove(OsStr::new(CONFIG));
        listing.config_entries.clear();
        listing.home = entries(&[(".config", link_to("/mnt/dotfiles/.config"))]);
        assert_eq!(
            removed(&plan(Path::new(DOTFILES), &listing)),
            [Path::new(".config")]
        );
    }

    #[test]
    fn takes_the_dotfiles_path_with_a_trailing_slash() {
        let mut listing = listing();
        listing.home = entries(&[(".zshrc", link_to("/mnt/dotfiles/.zshrc"))]);
        assert_eq!(
            plan(Path::new("/mnt/dotfiles/"), &listing).linked,
            [path(".zshrc")]
        );
    }

    #[test]
    fn prints_a_line_per_change_and_conflict() {
        let paint = Paint::plain();
        let changes = [
            Change::ConfigDirectory,
            linking(".zshrc"),
            Change::Repoint {
                target: path(".config/nvim"),
                entry: path("/mnt/dotfiles/.config/nvim"),
                from: path("/mnt/dotfiles/nvim"),
            },
            Change::Remove {
                target: path(".vimrc"),
                gone: path("/mnt/dotfiles/.vimrc"),
            },
        ];
        let text: String = changes.iter().map(|change| made(change, paint)).collect();
        assert_eq!(
            text,
            "linked    ~/.zshrc\nrepointed ~/.config/nvim\nremoved   ~/.vimrc\n"
        );
        let conflict = Conflict {
            target: path(".gitconfig"),
            found: link_to("/home/dev/git/config"),
        };
        assert_eq!(
            conflicting(&conflict, paint),
            "conflict  ~/.gitconfig links to /home/dev/git/config\n"
        );
    }

    #[test]
    fn shows_each_path_and_what_to_do() {
        let plan = Plan {
            linked: vec![path(".config/nvim")],
            changes: vec![
                Change::ConfigDirectory,
                linking(".zshrc"),
                Change::Repoint {
                    target: path(".tmux.conf"),
                    entry: path("/mnt/dotfiles/.tmux.conf"),
                    from: path("/mnt/dotfiles/tmux.conf"),
                },
                Change::Remove {
                    target: path(".vimrc"),
                    gone: path("/mnt/dotfiles/.vimrc"),
                },
            ],
            conflicts: vec![Conflict {
                target: path(".gitconfig"),
                found: Kind::File,
            }],
        };
        let home = Path::new("/home/dev");
        assert_eq!(
            render(Path::new(DOTFILES), home, &plan, Paint::plain()),
            "Dotfiles  /mnt/dotfiles\n\
             Home      /home/dev\n\
             \n\
             linked    ~/.config/nvim\n\
             missing   ~/.zshrc\n\
             missing   ~/.tmux.conf links to /mnt/dotfiles/tmux.conf\n\
             stale     ~/.vimrc links to /mnt/dotfiles/.vimrc\n\
             conflict  ~/.gitconfig is a file\n\
             \n\
             Open a new shell, or run lmx dotfiles link, to update the links.\n\
             Move the conflicting paths aside, then open a new shell.\n"
        );

        let settled = Plan {
            linked: vec![path(".zshrc")],
            ..Plan::default()
        };
        assert!(
            render(Path::new(DOTFILES), home, &settled, Paint::plain())
                .ends_with("\n\nlinked    ~/.zshrc\n")
        );
        assert!(
            render(Path::new(DOTFILES), home, &Plan::default(), Paint::plain())
                .ends_with("\n\nThe dotfiles have nothing to link.\n")
        );
    }

    #[test]
    fn the_doctor_counts_the_links_and_names_what_is_in_the_way() {
        let dotfiles = Path::new(DOTFILES);
        let summary = |plan: Plan| {
            let check = finding(dotfiles, &Ok(plan));
            (check.status, check.message, check.hint)
        };
        assert_eq!(
            summary(Plan {
                linked: vec![path(".zshrc"), path(".config/nvim")],
                changes: vec![Change::Remove {
                    target: path(".vimrc"),
                    gone: path("/mnt/dotfiles/.vimrc"),
                }],
                ..Plan::default()
            }),
            (
                CheckStatus::Ok,
                "2 dotfiles from /mnt/dotfiles are linked.".to_owned(),
                None
            )
        );
        assert_eq!(
            summary(Plan {
                linked: vec![path(".zshrc")],
                changes: vec![Change::ConfigDirectory, linking(".config/nvim")],
                ..Plan::default()
            })
            .1,
            "1 dotfile from /mnt/dotfiles is linked; the next shell links 1 more."
        );
        assert_eq!(
            summary(Plan::default()).1,
            "The dotfiles at /mnt/dotfiles have nothing to link."
        );
        let conflict = |target: &str| Conflict {
            target: path(target),
            found: Kind::File,
        };
        assert_eq!(
            summary(Plan {
                linked: vec![path(".zshrc")],
                conflicts: vec![conflict(".gitconfig"), conflict(".config/git")],
                ..Plan::default()
            }),
            (
                CheckStatus::Warning,
                "~/.gitconfig and ~/.config/git are in the way of the dotfiles from /mnt/dotfiles."
                    .to_owned(),
                Some("Move them aside, then open a new shell.".to_owned())
            )
        );
        assert_eq!(
            summary(Plan {
                conflicts: vec![conflict(".gitconfig")],
                ..Plan::default()
            })
            .2
            .as_deref(),
            Some("Move it aside, then open a new shell.")
        );

        let unreadable = finding(
            dotfiles,
            &Err(ReadError::Dotfiles {
                path: dotfiles.to_path_buf(),
                source: io::Error::from(io::ErrorKind::NotFound),
            }),
        );
        assert_eq!(unreadable.status, CheckStatus::Failed);
        assert!(
            unreadable
                .message
                .starts_with("cannot read the dotfiles at /mnt/dotfiles: "),
            "{}",
            unreadable.message
        );
        assert_eq!(
            unreadable.hint.as_deref(),
            Some("The host mounts the directory; limanix update on the Mac mounts it again.")
        );
    }

    #[test]
    fn reads_the_dotfiles_and_the_home() {
        let root = tempfile::tempdir().expect("temporary root");
        let (dotfiles, home) = (root.path().join("dotfiles"), root.path().join("home"));
        fs::create_dir_all(dotfiles.join(".config/nvim")).expect("create the dotfiles");
        fs::write(dotfiles.join(".zshrc"), "").expect("write .zshrc");
        fs::create_dir_all(home.join(".config")).expect("create the home");
        fs::write(home.join(".bashrc"), "").expect("write .bashrc");
        symlink(dotfiles.join(".zshrc"), home.join(".zshrc")).expect("link .zshrc");

        let listing = read(&dotfiles, &home).expect("readable");
        assert_eq!(listing.entries, set(&[".config", ".zshrc"]));
        assert_eq!(listing.config_entries, set(&["nvim"]));
        assert_eq!(
            listing.home,
            entries(&[
                (".bashrc", Kind::File),
                (".config", Kind::Directory),
                (".zshrc", Kind::Link(dotfiles.join(".zshrc"))),
            ])
        );
        assert!(listing.config_home.is_empty());

        fs::remove_dir_all(dotfiles.join(".config")).expect("remove .config");
        fs::write(dotfiles.join(".config"), "").expect("write a .config file");
        assert!(
            read(&dotfiles, &home)
                .expect("readable")
                .config_entries
                .is_empty()
        );

        let error = read(&root.path().join("missing"), &home).expect_err("no dotfiles");
        assert!(matches!(error, ReadError::Dotfiles { .. }), "{error}");
        let mountinfo = mount_table(root.path(), &dotfiles);
        let error = survey(Path::new("dotfiles"), &home, &mountinfo).expect_err("a relative path");
        assert!(matches!(error, ReadError::NotAbsolute(_)), "{error}");
    }

    fn mount_table(root: &Path, dotfiles: &Path) -> PathBuf {
        let path = root.join("mountinfo");
        fs::write(
            &path,
            format!(
                "22 1 254:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw\n\
                 42 22 0:40 / {} rw,relatime shared:22 - virtiofs mount2 rw\n",
                dotfiles.display()
            ),
        )
        .expect("write the mount table");
        path
    }

    #[test]
    fn changes_nothing_while_the_dotfiles_are_not_mounted() {
        let root = tempfile::tempdir().expect("temporary root");
        let (dotfiles, home) = (root.path().join("dotfiles"), root.path().join("home"));
        fs::create_dir_all(&dotfiles).expect("create the empty mount point");
        fs::create_dir_all(&home).expect("create the home");
        symlink(dotfiles.join(".zshrc"), home.join(".zshrc")).expect("link .zshrc");

        let unmounted = root.path().join("unmounted");
        fs::write(
            &unmounted,
            "22 1 254:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw\n",
        )
        .expect("write the mount table");
        let error = survey(&dotfiles, &home, &unmounted).expect_err("not mounted");
        assert!(matches!(error, ReadError::NotMounted(_)), "{error}");
        assert_eq!(
            error.to_string(),
            format!("the dotfiles at {} are not mounted", dotfiles.display())
        );
        let missing =
            survey(&dotfiles, &home, &root.path().join("missing")).expect_err("no mount table");
        assert!(matches!(missing, ReadError::MountTable(_)), "{missing}");

        let plan = survey(&dotfiles, &home, &mount_table(root.path(), &dotfiles)).expect("mounted");
        assert_eq!(
            plan.changes,
            [Change::Remove {
                target: path(".zshrc"),
                gone: dotfiles.join(".zshrc"),
            }]
        );

        let finding = finding(&dotfiles, &Err(error));
        assert_eq!(finding.status, CheckStatus::Failed);
        assert_eq!(
            finding.hint.as_deref(),
            Some("The host mounts the directory; limanix update on the Mac mounts it again.")
        );
    }

    #[test]
    fn makes_each_change_once_when_runs_overlap() {
        let root = tempfile::tempdir().expect("temporary root");
        let (dotfiles, home) = (root.path().join("dotfiles"), root.path().join("home"));
        fs::create_dir_all(dotfiles.join(".config/nvim")).expect("create the dotfiles");
        fs::write(dotfiles.join(".zshrc"), "").expect("write .zshrc");
        fs::write(dotfiles.join(".tmux.conf"), "").expect("write .tmux.conf");
        fs::create_dir_all(&home).expect("create the home");
        symlink(dotfiles.join(".vimrc"), home.join(".vimrc")).expect("link a removed entry");
        symlink(dotfiles.join("tmux.conf"), home.join(".tmux.conf")).expect("link elsewhere");

        let mountinfo = mount_table(root.path(), &dotfiles);
        let plan = survey(&dotfiles, &home, &mountinfo).expect("readable");
        assert_eq!(plan.changes.len(), 5, "{plan:?}");
        for change in &plan.changes {
            assert!(apply(&home, change).expect("made"), "{change:?}");
        }
        for change in &plan.changes {
            assert!(!apply(&home, change).expect("no failure"), "{change:?}");
        }
        let plan = survey(&dotfiles, &home, &mountinfo).expect("readable");
        assert!(
            plan.changes.is_empty() && plan.conflicts.is_empty(),
            "{plan:?}"
        );
        assert_eq!(
            plan.linked,
            [path(".tmux.conf"), path(".zshrc"), path(".config/nvim")]
        );
    }
}
