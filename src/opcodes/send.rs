use std::boxed::Box;
use std::ffi::c_void;
use std::os::fd::{AsRawFd, RawFd};

use crate::opcodes::SqeOp;

use liburing_sys::{io_uring_prep_send, io_uring_sqe};

pub struct Send {
    fd: RawFd,
    buf: Box<[u8]>,
    flags: i32,
}

impl Send {
    pub fn new(fd: &impl AsRawFd, buf: &[u8], flags: i32) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            buf: Box::from(buf),
            flags,
        }
    }
}

impl SqeOp for Send {
    fn apply(self, sqe: &mut io_uring_sqe) {
        unsafe {
            io_uring_prep_send(
                sqe,
                self.fd.as_raw_fd(),
                self.buf.as_ptr() as *const c_void,
                self.buf.len(),
                self.flags,
            );
        }
    }
}
