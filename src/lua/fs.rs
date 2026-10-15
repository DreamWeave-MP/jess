//! `jess.fs`: plain file access for scripts, within what the runtime's policy grants.
//!
//! Every function that touches the host's files needs `filesystem.read` or `filesystem.write`,
//! the capabilities `@dream/fs` needs; without it the function is bound to a permission error.
//! The functions that only take a path apart or join one are always there.

use std::path::{Path, PathBuf};

use crate::lua::files::{Backup, replace_file, write_file_with};
use l3i::Runtime;
use l3i::bind::{Call, StackResults, VarArgs};
use l3i::fs::{READ_CAPABILITY as READ, WRITE_CAPABILITY as WRITE};
use l3i::stack::Scope;
use l3i::value::Table;

use crate::lua::plan::Functions;
use crate::lua::{Bytes, Opts, Text, error, lua};

/// The module, in a runtime whose policy grants reading (`read`) and writing (`write`) or not.
#[allow(clippy::too_many_lines)]
pub fn module(runtime: &Runtime, read: bool, write: bool) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "fs")?;
    module.gated("read", READ, read, |path: Text| -> l3i::Result<Option<Vec<u8>>> {
        match std::fs::read(&path.0) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(error(format!("failed to read '{}': {source}", path.0))),
        }
    })?;
    // The same file as a buffer, read straight into the buffer the VM allocates: no Luau string is
    // made (or interned), and no second copy.
    module.gated(
        "readBuffer",
        READ,
        read,
        |call: &Call<'_>, path: Text| -> l3i::Result<StackResults> {
            use std::io::Read as _;
            let failed = |source: std::io::Error| error(format!("failed to read '{}': {source}", path.0));
            let mut file = match std::fs::File::open(&path.0) {
                Ok(file) => file,
                // No result is nil to the caller.
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(StackResults),
                Err(source) => return Err(failed(source)),
            };
            let len = usize::try_from(file.metadata().map_err(failed)?.len())
                .map_err(|_| error(format!("failed to read '{}': too large", path.0)))?;
            let mut buffer = l3i::convert::new_buffer(call, len)?;
            // SAFETY: the buffer was just created by this call and nothing else can see it yet.
            file.read_exact(unsafe { buffer.bytes_mut_unchecked() }).map_err(failed)?;
            Ok(StackResults)
        },
    )?;
    module.gated(
        "write",
        WRITE,
        write,
        |call: &Call<'_>, path: Text, data: Bytes, options: Opts| -> l3i::Result<StackResults> {
            options.check(call, "jess.fs.write", &["backup"])?;
            let bytes = data.0;
            let backup = match options.get::<bool>(call, "backup")? {
                Some(true) => Backup::Keep,
                _ => Backup::Skip,
            };
            let written = write_file_with(Path::new(&path.0), &bytes, backup).map_err(error)?;
            lua::push_table(call, 0, 2, |frame, table| {
                table.raw_set_value(frame, "path", &written.path.display().to_string())?;
                table.raw_set_value(
                    frame,
                    "backup",
                    &written.backup.as_ref().map(|path| path.display().to_string()),
                )
            })?;
            Ok(StackResults)
        },
    )?;
    module.gated("replace", WRITE, write, |path: Text, data: Bytes| {
        replace_file(Path::new(&path.0), &data.0).map_err(|source| error(format!("failed to write '{}': {source}", path.0)))
    })?;
    module.gated("copy", WRITE, write, |from: Text, to: Text| {
        std::fs::copy(&from.0, &to.0)
            .map(|_| ())
            .map_err(|source| error(format!("failed to copy '{}' to '{}': {source}", from.0, to.0)))
    })?;
    module.gated("remove", WRITE, write, |path: Text| -> l3i::Result<bool> {
        match std::fs::remove_file(&path.0) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(error(format!("failed to remove '{}': {source}", path.0))),
        }
    })?;
    module.gated("createDir", WRITE, write, |path: Text| {
        std::fs::create_dir_all(&path.0).map_err(|source| error(format!("failed to create '{}': {source}", path.0)))
    })?;
    module.gated("exists", READ, read, |path: Text| Path::new(&path.0).exists())?;
    module.gated("isDir", READ, read, |path: Text| Path::new(&path.0).is_dir())?;
    module.gated("isFile", READ, read, |path: Text| Path::new(&path.0).is_file())?;
    module.gated("size", READ, read, |path: Text| {
        std::fs::metadata(&path.0).ok().map(|metadata| metadata.len())
    })?;
    module.function("isFileName", |path: Text| {
        let mut components = Path::new(&path.0).components();
        matches!(components.next(), Some(std::path::Component::Normal(_))) && components.next().is_none()
    })?;
    module.gated(
        "stat",
        READ,
        read,
        |call: &Call<'_>, path: Text| -> l3i::Result<StackResults> {
            let Ok(metadata) = std::fs::metadata(&path.0) else {
                call.push(&())?;
                return Ok(StackResults);
            };
            let modified = metadata.modified().ok();
            let since = modified.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
            let mut frame = call.frame();
            let table = frame.push_table(0, 6)?;
            table.raw_set_value(&frame, "size", &metadata.len())?;
            table.raw_set_value(&frame, "isFile", &metadata.is_file())?;
            table.raw_set_value(&frame, "isDir", &metadata.is_dir())?;
            table.raw_set_value(
                &frame,
                "modified",
                &modified.map(|time| {
                    time.duration_since(std::time::UNIX_EPOCH)
                        .map_or_else(|before| -before.duration().as_secs_f64(), |after| after.as_secs_f64())
                }),
            )?;
            table.raw_set_value(&frame, "modifiedSeconds", &since.map(|time| time.as_secs()))?;
            table.raw_set_value(&frame, "modifiedNanoseconds", &since.map(|time| time.subsec_nanos()))?;
            frame.release();
            Ok(StackResults)
        },
    )?;
    module.gated("modified", READ, read, |path: Text| {
        let Ok(modified) = std::fs::metadata(&path.0).and_then(|metadata| metadata.modified()) else {
            return None;
        };
        let since = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_or_else(|before| -before.duration().as_secs_f64(), |after| after.as_secs_f64());
        Some(since)
    })?;
    module.gated(
        "list",
        READ,
        read,
        |call: &Call<'_>, path: Text| -> l3i::Result<StackResults> {
            let entries =
                std::fs::read_dir(&path.0).map_err(|source| error(format!("failed to list '{}': {source}", path.0)))?;
            let names = entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            lua::push_list(call, names.into_iter(), |frame, name| {
                frame.push_string(&name)?;
                Ok(())
            })?;
            Ok(StackResults)
        },
    )?;
    module.gated("cwd", READ, read, || -> l3i::Result<String> {
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .map_err(|source| error(format!("failed to read the working directory: {source}")))
    })?;
    module.function("join", |parts: VarArgs<Text>| {
        parts
            .values
            .iter()
            .map(|part| part.0.as_str())
            .collect::<PathBuf>()
            .display()
            .to_string()
    })?;
    module.function("fileName", |path: Text| {
        Path::new(&path.0).file_name().map(|name| name.to_string_lossy().into_owned())
    })?;
    module.function("parent", |path: Text| {
        Path::new(&path.0).parent().map(|parent| parent.display().to_string())
    })?;
    module.finish()
}
