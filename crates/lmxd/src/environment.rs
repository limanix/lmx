//! Environment files of a generation.
//!
//! The host writes the user's `[env]` into two files next to the generation's flake, so the values
//! stay out of the flake and the Nix store. Before a build they are installed into `/etc/limanix`,
//! where every service reads `environment` through a systemd drop-in and login shells source
//! `environment.sh`. Only root and the development account's group may read them.

use std::{
    fs::{self, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, chown},
};

use crate::paths::Paths;

/// Names of the environment files, the same in the generation and in `/etc/limanix`.
const FILES: [&str; 2] = ["environment", "environment.sh"];

/// Mode of the installed files: root writes, the group reads.
const FILE_MODE: u32 = 0o640;

/// Installs the generation's environment files into `/etc/limanix`.
///
/// The directory gets mode `0755`. Each file is written beside its target with mode `0640` and the
/// group `gid`, then renamed over it, so a reader never sees half a file.
pub(crate) fn install(paths: &Paths, gid: u32) -> io::Result<()> {
    let directory = paths.environment();
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, Permissions::from_mode(0o755))?;
    for name in FILES {
        let content = fs::read(paths.mount().join(name))?;
        let staged = directory.join(format!(".{name}.new"));
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&staged)?;
        file.write_all(&content)?;
        file.sync_all()?;
        // The creation mode is narrowed by the umask; set it exactly.
        fs::set_permissions(&staged, Permissions::from_mode(FILE_MODE))?;
        chown(&staged, None, Some(gid))?;
        fs::rename(&staged, directory.join(name))?;
    }
    Ok(())
}

/// Variables of an installed `environment` file, in file order.
///
/// The host writes one `NAME="value"` assignment per variable, escaping `\`, `"`, `$` and the
/// backtick with a backslash; a value may span lines. Anything else ends the parse, keeping the
/// variables read so far.
pub(crate) fn variables(text: &str) -> Vec<(String, String)> {
    let mut variables = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(['\n', '\r']);
        let Some((name, after)) = rest.split_once("=\"") else {
            return variables;
        };
        if name.is_empty() || name.contains(['\n', '"', '=']) {
            return variables;
        }
        let mut value = String::new();
        let mut characters = after.char_indices();
        let end = loop {
            match characters.next() {
                Some((_, '\\')) => match characters.next() {
                    Some((_, escaped)) => value.push(escaped),
                    None => return variables,
                },
                Some((index, '"')) => break index,
                Some((_, character)) => value.push(character),
                None => return variables,
            }
        };
        variables.push((name.to_owned(), value));
        rest = &after[end + 1..];
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn reads_the_assignments_the_host_writes() {
        let text = "HTTP_PROXY=\"http://proxy:3128\"\nGREETING=\"say \\\"hi\\\"\nto \\$USER\"\n";
        assert_eq!(
            variables(text),
            [
                ("HTTP_PROXY".to_owned(), "http://proxy:3128".to_owned()),
                ("GREETING".to_owned(), "say \"hi\"\nto $USER".to_owned()),
            ]
        );
        assert!(variables("").is_empty());
    }

    #[test]
    fn installs_the_files_for_root_and_the_group_only() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = Paths::new(root.path().to_path_buf());
        fs::create_dir_all(paths.mount()).expect("create the mount");
        fs::write(paths.mount().join("environment"), "A=\"1\"\n").expect("write");
        fs::write(paths.mount().join("environment.sh"), "export A=\"1\"\n").expect("write");
        let gid = fs::metadata(root.path()).expect("metadata").gid();

        install(&paths, gid).expect("install");
        for name in FILES {
            let installed = paths.environment().join(name);
            let metadata = fs::metadata(&installed).expect("installed");
            assert_eq!(metadata.mode() & 0o777, 0o640, "{name}");
            assert_eq!(metadata.gid(), gid, "{name}");
        }
        assert_eq!(
            fs::read_to_string(paths.environment().join("environment")).expect("read"),
            "A=\"1\"\n"
        );
    }
}
