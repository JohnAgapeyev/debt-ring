use liburing_sys::*;

//Same as an io_uring_cqe just without the user_data field
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrippedCqe {
    pub res: i32,
    pub flags: u32,
}

impl From<&io_uring_cqe> for StrippedCqe {
    fn from(cqe: &io_uring_cqe) -> Self {
        Self {
            res: cqe.res,
            flags: cqe.flags,
        }
    }
}
