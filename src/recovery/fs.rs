//! Descriptor-relative, owner-only recovery storage. Settings deliberately use
//! different symlink semantics; never replace their helpers with these.
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

pub(super) struct Directory(File);

fn name(value: &OsStr) -> io::Result<CString> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::other("invalid managed name"));
    }
    CString::new(bytes).map_err(io::Error::other)
}

fn owned(file: File, directory: bool) -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no arguments or ownership requirements.
    if metadata.uid() != unsafe { libc::geteuid() }
        || (directory && !metadata.is_dir())
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other(
            "unsafe recovery file ownership or permissions",
        ));
    }
    Ok(file)
}

impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("absolute root required"));
        }
        let mut current = File::open("/")?;
        for component in path.components() {
            let Component::Normal(part) = component else {
                if component == Component::RootDir {
                    continue;
                }
                return Err(io::Error::other("invalid root component"));
            };
            let name = name(part)?;
            // SAFETY: current owns the parent fd, name is NUL terminated.
            let mut fd = unsafe {
                libc::openat(
                    current.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd == -1 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
                // SAFETY: descriptor-relative create with restrictive initial mode.
                if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } == -1 {
                    return Err(io::Error::last_os_error());
                }
                current.sync_all()?;
                // SAFETY: same owned parent and validated name; never follows links.
                fd = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
            }
            if fd == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openat returned a fresh owned descriptor.
            current = unsafe { File::from_raw_fd(fd) };
        }
        Ok(Self(owned(current, true)?))
    }

    pub(super) fn child_self(&self) -> io::Result<Self> {
        // SAFETY: F_DUPFD_CLOEXEC returns an independently owned descriptor.
        let fd = unsafe { libc::fcntl(self.0.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the duplicate is transferred into File exactly once.
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    fn owned_here(&self, file: File, directory: bool) -> io::Result<File> {
        use std::os::unix::fs::MetadataExt;
        let file = owned(file, directory)?;
        if file.metadata()?.dev() != self.0.metadata()?.dev() {
            return Err(io::Error::other("managed recovery storage crosses devices"));
        }
        Ok(file)
    }

    pub(super) fn child(&self, value: &str, create: bool) -> io::Result<Self> {
        let child = name(OsStr::new(value))?;
        if create {
            // SAFETY: self owns fd and child is a validated single component.
            if unsafe { libc::mkdirat(self.0.as_raw_fd(), child.as_ptr(), 0o700) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: descriptor-relative directory open rejects links.
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                child.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: newly opened fd is owned here.
        Ok(Self(
            self.owned_here(unsafe { File::from_raw_fd(fd) }, true)?,
        ))
    }

    pub(super) fn file(&self, value: &str, create: bool) -> io::Result<File> {
        let value = name(OsStr::new(value))?;
        let flags = libc::O_RDWR
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if create {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            };
        // SAFETY: owned directory, validated name, restrictive creation mode.
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), value.as_ptr(), flags, 0o600) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fresh fd transferred into File exactly once.
        self.owned_here(unsafe { File::from_raw_fd(fd) }, false)
    }

    pub(super) fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        let from = name(OsStr::new(from))?;
        let to = name(OsStr::new(to))?;
        // Validate an existing target before replacing it; operations stay under
        // the locked private directory, and do not follow either target.
        match self.file(to.to_str().map_err(io::Error::other)?, false) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // SAFETY: names and owned fd remain live during the call.
        if unsafe {
            libc::renameat(
                self.0.as_raw_fd(),
                from.as_ptr(),
                self.0.as_raw_fd(),
                to.as_ptr(),
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn remove(&self, value: &str, directory: bool) -> io::Result<()> {
        if !directory {
            self.file(value, false)?;
        }
        let value = name(OsStr::new(value))?;
        // SAFETY: unlinkat removes this directory entry, never traverses a link.
        if unsafe {
            libc::unlinkat(
                self.0.as_raw_fd(),
                value.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn entries(&self) -> io::Result<Vec<String>> {
        // SAFETY: openat creates an independent directory cursor; dup would share
        // the seek offset with previous enumerations.
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fdopendir takes ownership on success.
        let directory = unsafe { libc::fdopendir(fd) };
        if directory.is_null() {
            // SAFETY: fdopendir failed, so fd remains ours.
            unsafe {
                libc::close(fd);
            }
            return Err(io::Error::last_os_error());
        }
        let mut entries = Vec::new();
        loop {
            // SAFETY: directory is live until closed below; copy the name now.
            // SAFETY: errno is thread-local on macOS; clear it to distinguish EOF.
            unsafe {
                *libc::__error() = 0;
            }
            let entry = unsafe { libc::readdir(directory) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    // SAFETY: close our live directory stream on this error path.
                    unsafe {
                        libc::closedir(directory);
                    }
                    return Err(error);
                }
                break;
            }
            // SAFETY: readdir supplies a NUL-terminated name in the live entry.
            let value = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if value.to_bytes() == b"." || value.to_bytes() == b".." {
                continue;
            }
            match value.to_str() {
                Ok(value) if entries.len() < 100_001 => entries.push(value.to_owned()),
                _ => {
                    // SAFETY: close exactly once before returning the error.
                    unsafe {
                        libc::closedir(directory);
                    }
                    return Err(io::Error::other("recovery directory exceeds bounds"));
                }
            }
        }
        // SAFETY: no outstanding references into this directory stream remain.
        if unsafe { libc::closedir(directory) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(entries)
    }

    /// Publish entries to the filesystem. A later same-device full barrier
    /// must complete before acknowledging the encompassing transaction.
    pub(super) fn sync_entries(&self) -> io::Result<()> {
        self.0.sync_all()
    }
    pub(super) fn sync(&self, barrier: &File) -> io::Result<()> {
        self.0.sync_all()?;
        full_sync(barrier)
    }

    pub(super) fn free_bytes(&self) -> io::Result<u64> {
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: fstatvfs initializes stats on success, self owns its fd.
        if unsafe { libc::fstatvfs(self.0.as_raw_fd(), stats.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fstatvfs succeeded above.
        let stats = unsafe { stats.assume_init() };
        Ok(u64::from(stats.f_bavail).saturating_mul(stats.f_frsize))
    }
}

pub(super) fn full_sync(file: &File) -> io::Result<()> {
    // SAFETY: macOS F_FULLFSYNC takes a live fd; the third argument is ignored.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
