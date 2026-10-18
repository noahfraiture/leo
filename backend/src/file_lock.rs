//! Filesystem writer ownership that ends when the application guard leaves scope.

use std::{
    fs::{File, TryLockError},
    io,
};

/// An exclusive writer lock whose lifetime is independent of duplicated or inherited descriptors.
#[derive(Debug)]
pub struct FileLock(File);

impl FileLock {
    /// Wait for exclusive ownership of an already opened lock file.
    pub fn acquire(file: File) -> io::Result<Self> {
        file.lock()?;
        Ok(Self(file))
    }

    /// Acquire exclusive ownership immediately, distinguishing contention from storage errors.
    pub fn try_acquire(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self(file))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Closing only this descriptor can leave a child holding the same OS lock until exec.
        if let Err(error) = self.0.unlock() {
            tracing::error!(%error, "filesystem writer lock release failed");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn independent_writers_remain_exclusive_until_the_owner_is_dropped() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        let open = || {
            File::options()
                .read(true)
                .write(true)
                .open(temporary.path())
                .unwrap()
        };
        let owner = FileLock::try_acquire(open()).unwrap();
        assert!(matches!(
            FileLock::try_acquire(open()),
            Err(TryLockError::WouldBlock)
        ));

        drop(owner);

        FileLock::try_acquire(open()).unwrap();
    }

    #[test]
    fn dropping_the_owner_releases_a_lock_while_a_duplicated_descriptor_remains_open() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        let open = || {
            File::options()
                .read(true)
                .write(true)
                .open(temporary.path())
                .unwrap()
        };
        let owner = FileLock::acquire(open()).unwrap();
        // Model a descriptor inherited by a child between process creation and exec.
        let inherited = owner.0.try_clone().unwrap();

        drop(owner);

        let next = FileLock::try_acquire(open()).expect("the previous writer finished");
        drop(inherited);
        assert!(matches!(
            FileLock::try_acquire(open()),
            Err(TryLockError::WouldBlock)
        ));
        drop(next);
    }
}
