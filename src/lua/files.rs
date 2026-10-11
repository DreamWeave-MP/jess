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
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
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
