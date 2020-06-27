//! Native FFI bindings to libfuse.
//!
//! This is a small set of bindings that are required to mount/unmount FUSE filesystems and
//! open/close a fd to the FUSE kernel driver.

#![warn(missing_debug_implementations, rust_2018_idioms)]
#![allow(missing_docs)]

use std::fmt;
use std::os::raw::{c_char, c_int, c_void};

#[repr(C)]
#[derive(Debug)]
pub struct fuse_args {
    pub argc: c_int,
    pub argv: *const *const c_char,
    pub allocated: c_int,
}

const OP_COUNT: usize = 44;

#[repr(C)]
pub struct fuse_lowlevel_op {
    _ptr: [usize; OP_COUNT],
}

impl fmt::Debug for fuse_lowlevel_op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("fuse_args").finish()
    }
}

impl fuse_lowlevel_op {
    pub fn new() -> Self {
        Self {
            _ptr: [0; OP_COUNT],
        }
    }
}

impl Default for fuse_lowlevel_op {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct fuse_session {
    _priv: [usize; 0],
}

extern "C" {
    // *_compat25 functions were introduced in FUSE 2.6 when function signatures changed.
    // Therefore, the minimum version requirement for *_compat25 functions is libfuse-2.6.0.

    pub fn fuse_session_new(
        args: *const fuse_args,
        op: *const fuse_lowlevel_op,
        op_size: usize,
        userdata: *mut c_void,
    ) -> *mut fuse_session;
    pub fn fuse_session_mount(se: *mut fuse_session, mountpoint: *const c_char) -> c_int;
    pub fn fuse_session_fd(se: *mut fuse_session) -> c_int;
    pub fn fuse_session_unmount(se: *mut fuse_session);
    pub fn fuse_session_destroy(se: *mut fuse_session);
}
