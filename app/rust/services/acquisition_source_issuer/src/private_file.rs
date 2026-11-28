//! Operator-private input/credential loader, pinned at every path component.
//! The separate issuer has no dependency on the PG operator service.

use std::{fs, path::Path};

use anyhow::{ensure, Context, Result};

pub(crate) fn read_private_file(path: &Path) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "issuer private path must be absolute");
    ensure!(
        path.components().all(|part| !matches!(
            part,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )),
        "issuer private path must not traverse relative components"
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

        let fd = unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "issuer cannot pin filesystem root");
        let mut dir = unsafe { fs::File::from_raw_fd(fd) };
        let mut parents = vec![dir.metadata()?];
        let names: Vec<_> = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        ensure!(!names.is_empty(), "issuer requires a file path");
        let mut final_file = None;
        for (index, name) in names.iter().enumerate() {
            let leaf = index + 1 == names.len();
            let encoded = CString::new(name.as_bytes())?;
            let flags = if leaf {
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
            };
            let fd = unsafe { libc::openat(dir.as_raw_fd(), encoded.as_ptr(), flags) };
            ensure!(
                fd >= 0,
                "issuer private path has unsafe or inaccessible component"
            );
            let opened = unsafe { fs::File::from_raw_fd(fd) };
            let metadata = opened.metadata()?;
            if leaf {
                ensure!(metadata.is_file(), "issuer input must be regular file");
                ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "issuer private file must have 0600/0400 permissions"
                );
                ensure!(
                    metadata.len() <= 2 * 1024 * 1024,
                    "issuer input is oversized"
                );
                final_file = Some((opened, metadata.uid()));
            } else {
                ensure!(metadata.is_dir(), "issuer parent must be directory");
                parents.push(metadata);
                dir = opened;
            }
        }
        let (mut file, owner) = final_file.context("issuer file descriptor unavailable")?;
        ensure!(
            parents.iter().all(|meta| {
                meta.is_dir()
                    && meta.permissions().mode() & 0o022 == 0
                    && (meta.uid() == 0 || meta.uid() == owner)
            }),
            "issuer parent is writable by group/world or has foreign owner"
        );
        let mut bytes = Vec::new();
        file.by_ref()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "issuer input grew beyond limit"
        );
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("private source issuance is available only with Unix no-follow file support")
    }
}
