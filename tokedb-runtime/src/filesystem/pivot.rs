#![allow(unsafe_code)]

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use nix::mount::{umount2, MntFlags};
use nix::unistd::chdir;

use crate::error::{Result, RuntimeError};

const PUT_OLD: &str = "/.old_root";

pub fn pivot_root(new_root: &Path) -> Result<()> {
    let put_old = new_root.join(PUT_OLD.trim_start_matches('/'));
    std::fs::create_dir_all(&put_old).map_err(|err| RuntimeError::Io {
        path: put_old.display().to_string(),
        message: err.to_string(),
    })?;

    let new_root_c =
        CString::new(new_root.as_os_str().as_bytes()).map_err(|_| RuntimeError::InvalidPath {
            path: new_root.display().to_string(),
            reason: "contains NUL",
        })?;
    let put_old_c =
        CString::new(put_old.as_os_str().as_bytes()).map_err(|_| RuntimeError::InvalidPath {
            path: put_old.display().to_string(),
            reason: "contains NUL",
        })?;

    let rc = unsafe {
        libc::syscall(
            libc::SYS_pivot_root,
            new_root_c.as_ptr(),
            put_old_c.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(RuntimeError::Process(format!(
            "pivot_root: {}",
            std::io::Error::last_os_error()
        )));
    }

    chdir("/").map_err(RuntimeError::from)?;
    umount2(Path::new(PUT_OLD), MntFlags::MNT_DETACH).map_err(RuntimeError::from)?;
    Ok(())
}
