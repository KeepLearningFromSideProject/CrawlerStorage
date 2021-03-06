//! FUSE kernel driver communication
//!
//! Raw communication channel to the FUSE kernel driver.

use fuse_sys::{
    fuse_args, fuse_lowlevel_op, fuse_session, fuse_session_fd, fuse_session_mount,
    fuse_session_new,
};
use libc::{self, c_int, c_void, size_t};
use log::error;
use std::ffi::{CString, OsStr};
use std::io;
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use crate::reply::ReplySender;

/// Helper function to provide options as a fuse_args struct
/// (which contains an argc count and an argv pointer)
fn with_fuse_args<T, F: FnOnce(&fuse_args) -> T>(options: &[&OsStr], f: F) -> T {
    let mut args = vec![CString::new("fuse-rs").unwrap()];
    args.extend(options.iter().map(|s| CString::new(s.as_bytes()).unwrap()));
    let argptrs: Vec<_> = args.iter().map(|s| s.as_ptr()).collect();
    f(&fuse_args {
        argc: argptrs.len() as i32,
        argv: argptrs.as_ptr(),
        allocated: 0,
    })
}

/// A raw communication channel to the FUSE kernel driver
#[derive(Debug)]
pub struct Channel {
    mountpoint: PathBuf,
    fd: c_int,
    se: *mut fuse_session,
}

/// # Safety: we could make sure pointer doesn't alias
unsafe impl Send for Channel {}

impl Channel {
    /// Create a new communication channel to the kernel driver by mounting the
    /// given path. The kernel driver will delegate filesystem operations of
    /// the given path to the channel. If the channel is dropped, the path is
    /// unmounted.
    pub fn new(mountpoint: &Path, options: &[&OsStr]) -> io::Result<Channel> {
        let mountpoint = mountpoint.canonicalize()?;
        with_fuse_args(options, |args| {
            let mnt = CString::new(mountpoint.as_os_str().as_bytes())?;
            let op = fuse_lowlevel_op::new();
            let se = unsafe {
                fuse_session_new(
                    args,
                    &op,
                    mem::size_of::<fuse_lowlevel_op>(),
                    ptr::null_mut(),
                )
            };
            if se.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "fuse_session_new fail",
                ));
            }
            let res = unsafe { fuse_session_mount(se, mnt.as_ptr()) };
            if res < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = unsafe { fuse_session_fd(se) };

            Ok(Channel { mountpoint, fd, se })
        })
    }

    /// Return path of the mounted filesystem
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Receives data up to the capacity of the given buffer (can block).
    pub fn receive(&self, buffer: &mut Vec<u8>) -> io::Result<()> {
        let rc = unsafe {
            libc::read(
                self.fd,
                buffer.as_ptr() as *mut c_void,
                buffer.capacity() as size_t,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            unsafe {
                buffer.set_len(rc as usize);
            }
            Ok(())
        }
    }

    /// Returns a sender object for this channel. The sender object can be
    /// used to send to the channel. Multiple sender objects can be used
    /// and they can safely be sent to other threads.
    pub fn sender(&self) -> ChannelSender {
        // Since write/writev syscalls are threadsafe, we can simply create
        // a sender by using the same fd and use it in other threads. Only
        // the channel closes the fd when dropped. If any sender is used after
        // dropping the channel, it'll return an EBADF error.
        ChannelSender { fd: self.fd }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        unsafe {
            fuse_sys::fuse_session_unmount(self.se);
            fuse_sys::fuse_session_destroy(self.se);
        }
        // TODO: send ioctl FUSEDEVIOCSETDAEMONDEAD on macOS before closing the fd
        // Close the communication channel to the kernel driver
        // (closing it before unnmount prevents sync unmount deadlock)
        // Unmount this channel's mount point
        // let _ = unmount(&self.mountpoint);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ChannelSender {
    fd: c_int,
}

impl ChannelSender {
    /// Send all data in the slice of slice of bytes in a single write (can block).
    pub fn send(&self, buffer: &[&[u8]]) -> io::Result<()> {
        let iovecs: Vec<_> = buffer
            .iter()
            .map(|d| libc::iovec {
                iov_base: d.as_ptr() as *mut c_void,
                iov_len: d.len() as size_t,
            })
            .collect();
        let rc = unsafe { libc::writev(self.fd, iovecs.as_ptr(), iovecs.len() as c_int) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl ReplySender for ChannelSender {
    fn send(&self, data: &[&[u8]]) {
        if let Err(err) = ChannelSender::send(self, data) {
            error!("Failed to send FUSE reply: {}", err);
        }
    }
}

pub fn unmount(mountpoint: &Path) -> io::Result<()> {
    Command::new("fusermount3")
        .args(&[
            OsStr::new("-q"),
            OsStr::new("-u"),
            OsStr::new("-z"),
            OsStr::new("--"),
            mountpoint.as_ref(),
        ])
        .status()?;
    Ok(())
}

// /// Unmount an arbitrary mount point
// pub fn unmount(mountpoint: &Path) -> io::Result<()> {
//     // fuse_unmount_compat22 unfortunately doesn't return a status. Additionally,
//     // it attempts to call realpath, which in turn calls into the filesystem. So
//     // if the filesystem returns an error, the unmount does not take place, with
//     // no indication of the error available to the caller. So we call unmount
//     // directly, which is what osxfuse does anyway, since we already converted
//     // to the real path when we first mounted.

//     #[cfg(any(
//         target_os = "macos",
//         target_os = "freebsd",
//         target_os = "dragonfly",
//         target_os = "openbsd",
//         target_os = "bitrig",
//         target_os = "netbsd"
//     ))]
//     #[inline]
//     fn libc_umount(mnt: &CStr) -> c_int {
//         unsafe {
//             libc::close(self.fd);
//         }

//         unsafe { libc::unmount(mnt.as_ptr(), 0) }
//     }

//     #[cfg(not(any(
//         target_os = "macos",
//         target_os = "freebsd",
//         target_os = "dragonfly",
//         target_os = "openbsd",
//         target_os = "bitrig",
//         target_os = "netbsd"
//     )))]
//     #[inline]
//     fn libc_umount(mnt: &CStr) -> c_int {
//         use fuse_sys::fuse_unmount_compat22;
//         use std::io::ErrorKind::PermissionDenied;

//         let rc = unsafe { libc::umount(mnt.as_ptr()) };
//         if rc < 0 && io::Error::last_os_error().kind() == PermissionDenied {
//             // Linux always returns EPERM for non-root users.  We have to let the
//             // library go through the setuid-root "fusermount -u" to unmount.
//             unsafe {
//                 fuse_unmount_compat22(mnt.as_ptr());
//             }
//             0
//         } else {
//             rc
//         }
//     }

//     let mnt = CString::new(mountpoint.as_os_str().as_bytes())?;
//     let rc = libc_umount(&mnt);
//     if rc < 0 {
//         Err(io::Error::last_os_error())
//     } else {
//         Ok(())
//     }
// }

#[cfg(test)]
mod test {
    use super::with_fuse_args;
    use std::ffi::{CStr, OsStr};

    #[test]
    fn fuse_args() {
        with_fuse_args(&[OsStr::new("foo"), OsStr::new("bar")], |args| {
            assert_eq!(args.argc, 3);
            assert_eq!(
                unsafe { CStr::from_ptr(*args.argv.offset(0)).to_bytes() },
                b"fuse-rs"
            );
            assert_eq!(
                unsafe { CStr::from_ptr(*args.argv.offset(1)).to_bytes() },
                b"foo"
            );
            assert_eq!(
                unsafe { CStr::from_ptr(*args.argv.offset(2)).to_bytes() },
                b"bar"
            );
        });
    }
}
