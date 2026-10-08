//! Shared folders: the virtiofs and 9p mounts of the guest.

use std::{fs, path::Path};

use crate::FactError;

/// Mount table of the calling process inside a booted guest.
pub const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// File-system types that carry shared folders.
const SHARED_TYPES: [&str; 2] = ["virtiofs", "9p"];

/// One shared folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Mount point in the guest.
    pub target: String,
    /// File-system type: `virtiofs` or `9p`.
    pub fs_type: String,
    /// Whether the guest can only read the folder.
    pub read_only: bool,
}

/// Reads the shared folders from the mount table at `path`.
///
/// A mount point that is not UTF-8 does not hide the others: its invalid bytes become U+FFFD.
pub fn shared(path: &Path) -> Result<Vec<Mount>, FactError> {
    let table = fs::read(path).map_err(|source| FactError::Io {
        what: "the mount table",
        source,
    })?;
    parse(&String::from_utf8_lossy(&table))
}

/// Parses `/proc/self/mountinfo`, keeping virtiofs and 9p mounts.
pub fn parse(table: &str) -> Result<Vec<Mount>, FactError> {
    let mut mounts = Vec::new();
    for line in table.lines().filter(|line| !line.trim().is_empty()) {
        let malformed = || FactError::Parse {
            what: "mount table",
            detail: line.to_owned(),
        };
        let (head, tail) = line.split_once(" - ").ok_or_else(malformed)?;
        let fields: Vec<&str> = head.split(' ').collect();
        let (Some(target), Some(options)) = (fields.get(4), fields.get(5)) else {
            return Err(malformed());
        };
        let mut tail = tail.split(' ');
        let fs_type = tail.next().unwrap_or_default();
        let super_options = tail.nth(1).unwrap_or_default();
        if SHARED_TYPES.contains(&fs_type) {
            mounts.push(Mount {
                target: unescape(target),
                fs_type: fs_type.to_owned(),
                read_only: has_ro(options) || has_ro(super_options),
            });
        }
    }
    Ok(mounts)
}

/// Whether comma-separated mount options include `ro`.
fn has_ro(options: &str) -> bool {
    options.split(',').any(|option| option == "ro")
}

/// Decodes the kernel's three-digit octal escapes; anything else is kept as written.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    
    while index < bytes.len() {
        let escaped = (bytes[index] == b'\\')
            .then(|| bytes.get(index + 1..index + 4))
            .flatten()
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 4;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn keeps_shared_folders_with_their_mode() {
        let table = "\
22 1 252:2 / / rw,noatime shared:1 - ext4 /dev/vda2 rw
41 22 0:38 / /home/dev rw,relatime shared:20 - virtiofs mount0 rw
42 22 0:39 / /mnt/limanix ro,relatime shared:21 - virtiofs mount1 rw
43 22 0:40 / /work\\040space rw,relatime - 9p mount2 rw,trans=virtio
44 22 0:41 / /srv/shared rw,relatime - virtiofs mount3 ro
";
        assert_eq!(
            parse(table).expect("valid mount table"),
            [
                Mount {
                    target: "/home/dev".into(),
                    fs_type: "virtiofs".into(),
                    read_only: false
                },
                Mount {
                    target: "/mnt/limanix".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true
                },
                Mount {
                    target: "/work space".into(),
                    fs_type: "9p".into(),
                    read_only: false
                },
                Mount {
                    target: "/srv/shared".into(),
                    fs_type: "virtiofs".into(),
                    read_only: true
                },
            ]
        );
    }

    #[test]
    fn keeps_text_that_is_not_an_escape() {
        assert_eq!(unescape(r"/a\b\0x9\400"), r"/a\b\0x9\400");
        assert_eq!(unescape(r"/tab\011and\134slash"), "/tab\tand\\slash");
        assert_eq!(unescape(r"/a\+12b"), r"/a\+12b");
    }

    #[test]
    fn keeps_shared_folders_beside_a_mount_point_that_is_not_utf8() {
        let mut table = tempfile::NamedTempFile::new().expect("temporary mount table");
        table
            .write_all(
                b"\
30 22 8:1 / /media/caf\xe9 rw - ext4 /dev/sdb1 rw
41 22 0:38 / /home/dev rw - virtiofs mount0 rw
",
            )
            .expect("write the mount table");
        let mounts = shared(table.path()).expect("readable mount table");
        assert_eq!(
            mounts
                .iter()
                .map(|mount| mount.target.as_str())
                .collect::<Vec<_>>(),
            ["/home/dev"]
        );
    }

    #[test]
    fn rejects_lines_without_a_separator() {
        let error = parse("22 1 252:2 / / rw ext4 /dev/vda2 rw").expect_err("no separator");
        assert!(
            error
                .to_string()
                .starts_with("unexpected mount table output"),
            "{error}"
        );
    }

    #[test]
    fn explains_an_unreadable_table() {
        let error = shared(Path::new("/nonexistent/mountinfo")).expect_err("missing table");
        assert!(
            error
                .to_string()
                .starts_with("cannot read the mount table: "),
            "{error}"
        );
    }
}
