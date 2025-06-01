use crate::opcodes::SqeOp;

use liburing_sys::{io_uring_prep_nop, io_uring_sqe};

pub struct Nop;

impl Nop {
    pub fn new() -> Self {
        Self {}
    }
}

impl SqeOp for Nop {
    fn apply(self, sqe: &mut io_uring_sqe) {
        unsafe {
            io_uring_prep_nop(sqe);
        }
    }
}
