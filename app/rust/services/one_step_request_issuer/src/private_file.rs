//! Pinned, no-follow Unix private input/credential loading, independent of
//! other owner binaries. Symlinks, FIFOs, writable parents and group-readable
//! credentials must never be accepted by a privileged request issuer.

use std::{fs, path::Path};

use anyhow::{ensure, Context, Result};

pub(crate) fn read_private_file(path: &Path) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "private issuer path must be absolute");
    ensure!(
        path.components().all(|part| !matches!(
            part,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )),
        "private issuer path cannot traverse relative components"
    );
    #[cfg(unix)]
    {
        use std::{
            ffi::CString,
            io::Read,
            os::{
                fd::{AsRawFd, FromRawFd},
                unix::{
                    ffi::OsStrExt,
                    fs::{MetadataExt, PermissionsExt},
                },
            },
            path::Component,
        };

        // SAFETY: valid static C string; the successfully opened descriptor
        // transfers exactly once to File and is then closed on drop.
        let fd = unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "cannot pin private issuer filesystem root");
        let mut directory = unsafe { fs::File::from_raw_fd(fd) };
        let mut parents = vec![directory.metadata()?];
        let names: Vec<_> = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        ensure!(!names.is_empty(), "issuer requires a regular private file");
        let mut opened_file = None;
        for (index, name) in names.iter().enumerate() {
            let leaf = index + 1 == names.len();
            let encoded = CString::new(name.as_bytes())?;
            let flags = if leaf {
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
            };
            let fd = unsafe { libc::openat(directory.as_raw_fd(), encoded.as_ptr(), flags) };
            ensure!(fd >= 0, "unsafe or inaccessible issuer private path");
            let opened = unsafe { fs::File::from_raw_fd(fd) };
            let metadata = opened.metadata()?;
            if leaf {
                ensure!(metadata.is_file(), "issuer private input must be a file");
                ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "issuer private input must have 0600/0400 permissions"
                );
                ensure!(metadata.len() <= 2 * 1024 * 1024, "oversized private input");
                opened_file = Some((opened, metadata.uid()));
            } else {
                ensure!(metadata.is_dir(), "issuer private parent must be a directory");
                parents.push(metadata);
                directory = opened;
            }
        }
        let (mut file, owner) = opened_file.context("private issuer file missing")?;
        ensure!(
            parents.iter().all(|meta| {
                meta.is_dir()
                    && meta.permissions().mode() & 0o022 == 0
                    && (meta.uid() == 0 || meta.uid() == owner)
            }),
            "issuer private parent must not be group/world-writable or foreign-owned"
        );
        let mut bytes = Vec::new();
        file.by_ref().take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 2 * 1024 * 1024, "issuer input grew beyond limit");
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("private request grant issuance requires Unix no-follow support")
    }
}
