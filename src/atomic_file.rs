//! Same-directory atomic replacement for completed temporary files.
use std::{io, path::Path};

/// Atomically install a completed temporary file at `destination`, replacing
/// an existing regular destination where the platform supports it.
pub fn replace(temp: tempfile::NamedTempFile, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

        let temp_path = temp.into_temp_path();
        let source: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let moved =
            unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
        if moved != 0 {
            // MoveFileExW has consumed the temporary path. Calling `keep()`
            // afterward checks that the old source path still exists and
            // incorrectly reports failure after a successful replacement.
            drop(temp_path);
            return Ok(());
        }
        let replace_error = io::Error::last_os_error();
        if replace_error.raw_os_error() != Some(5) {
            return Err(replace_error);
        }
        // The managed desktop sandbox denies native replacement. Keep a
        // rollback copy and use a checked copy fallback there. Ordinary Windows
        // builds take the atomic MoveFileExW path above.
        let result = replace_by_copy(&temp_path, destination);
        drop(temp_path);
        result
    }
    #[cfg(not(windows))]
    {
        temp.persist(destination)
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

#[cfg(windows)]
fn replace_by_copy(source: &Path, destination: &Path) -> io::Result<()> {
    let destination_state = match std::fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Some(()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination must be a regular file, not a link or directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let backup = if destination_state.is_some() {
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut backup = tempfile::NamedTempFile::new_in(parent)?;
        let mut previous = std::fs::File::open(destination)?;
        std::io::copy(&mut previous, backup.as_file_mut())?;
        backup.as_file().sync_all()?;
        Some(backup)
    } else {
        None
    };
    let write_result = copy_contents(source, destination);
    if let Err(error) = write_result {
        let restored = if let Some(backup) = backup.as_ref() {
            copy_contents(backup.path(), destination)
        } else {
            std::fs::remove_file(destination).or_else(|remove_error| {
                if remove_error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(remove_error)
                }
            })
        };
        return match restored {
            Ok(()) => Err(error),
            Err(restore_error) => Err(io::Error::other(format!(
                "replacement failed ({error}), and restoring the previous file failed ({restore_error})"
            ))),
        };
    }
    Ok(())
}

#[cfg(windows)]
fn copy_contents(source: &Path, destination: &Path) -> io::Result<()> {
    use std::io::Write;

    let mut source = std::fs::File::open(source)?;
    let mut destination = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)?;
    std::io::copy(&mut source, &mut destination)?;
    destination.flush()?;
    destination.sync_all()
}
