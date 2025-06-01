pub mod nop;

use liburing_sys::io_uring_sqe;

pub trait SqeOp {
    fn apply(self, sqe: &mut io_uring_sqe);
}
