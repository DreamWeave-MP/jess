//! Generic host file writes shared by package tooling, initialization, and script IO.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum Backup {
    Keep,
    Skip,
}

/// A completed host file write and its optional backup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Written {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
}

/// Replaces a file through a unique sibling temporary, preserving existing permissions.
///
/// A file that wasn't there gets the permissions creating it directly would have given it: the
/// temporary's own are owner-only, which is right for a temporary and wrong for what a tool
/// writes for other programs to read.
pub fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut builder = tempfile::Builder::new();
    // The mode is what `File::create` asks for, and the process's umask applies to it as it
    // would there.
    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder.tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Backs up an existing target without clobbering an earlier backup, then replaces it.
pub fn write_file_with(path: &Path, bytes: &[u8], backup: Backup) -> io::Result<Written> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let backup = if matches!(backup, Backup::Keep) {
        match fs::File::open(path) {
            Ok(mut original) => {
                let mut index = 0u64;
                loop {
                    let suffix = if index == 0 {
                        ".bak".to_owned()
                    } else {
                        format!(".bak{index}")
                    };
                    let mut name = path.as_os_str().to_os_string();
                    name.push(suffix);
                    let candidate = PathBuf::from(name);
                    match OpenOptions::new().write(true).create_new(true).open(&candidate) {
                        Ok(mut copy) => {
                            let result = io::copy(&mut original, &mut copy).and_then(|_| copy.sync_all());
                            if let Err(error) = result {
                                drop(copy);
                                drop(fs::remove_file(&candidate));
                                return Err(error);
                            }
                            break Some(candidate);
                        }
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => index += 1,
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    replace_file(path, bytes)?;
    Ok(Written {
        path: path.to_path_buf(),
        backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_are_never_overwritten() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("nested/file.bin");
        write_file_with(&path, b"first", Backup::Keep).unwrap();
        let first = write_file_with(&path, b"second", Backup::Keep).unwrap();
        let second = write_file_with(&path, b"third", Backup::Keep).unwrap();
        assert_eq!(fs::read(first.backup.unwrap()).unwrap(), b"first");
        assert_eq!(fs::read(second.backup.unwrap()).unwrap(), b"second");
        assert_eq!(fs::read(&path).unwrap(), b"third");
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_gets_the_permissions_creating_it_would_and_an_existing_one_keeps_its_own() {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().unwrap();
        let created = folder.path().join("created.bin");
        fs::File::create(&created).unwrap();
        let expected = fs::metadata(&created).unwrap().permissions().mode();

        let written = folder.path().join("written.bin");
        replace_file(&written, b"bytes").unwrap();
        assert_eq!(fs::metadata(&written).unwrap().permissions().mode(), expected);

        fs::set_permissions(&written, fs::Permissions::from_mode(0o640)).unwrap();
        replace_file(&written, b"again").unwrap();
        assert_eq!(fs::metadata(&written).unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[test]
    fn failed_replacement_leaves_the_target_and_no_temporary() {
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path().join("directory");
        fs::create_dir(&target).unwrap();
        assert!(replace_file(&target, b"bytes").is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
    }
}
