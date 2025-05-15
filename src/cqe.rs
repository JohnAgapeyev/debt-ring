use liburing_sys::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CqeFlag {
    BufferId(u16),
    More,
    SockNonEmpty,
    Notification,
    BufMore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CqeFlags(u32);

impl CqeFlags {
    //If set, the upper 16 bits of the flags field carries the buffer ID that was chosen for this request.
    pub fn buffer_id(&self) -> Option<u16> {
        if (self.0 & IORING_CQE_F_BUFFER) == 0 {
            return None;
        }
        Some((self.0 >> 16).try_into().unwrap())
    }
    pub fn more(&self) -> Option<()> {
        if (self.0 & IORING_CQE_F_MORE) == 0 {
            return None;
        }
        Some(())
    }
    pub fn sock_nonempty(&self) -> Option<()> {
        if (self.0 & IORING_CQE_F_SOCK_NONEMPTY) == 0 {
            return None;
        }
        Some(())
    }
    pub fn notification(&self) -> Option<()> {
        if (self.0 & IORING_CQE_F_NOTIF) == 0 {
            return None;
        }
        Some(())
    }
    pub fn buf_more(&self) -> Option<()> {
        if (self.0 & IORING_CQE_F_BUF_MORE) == 0 {
            return None;
        }
        Some(())
    }
    pub fn contains(&self, flag: CqeFlag) -> bool {
        match flag {
            CqeFlag::BufferId(_) => self.buffer_id().is_some(),
            CqeFlag::More => self.more().is_some(),
            CqeFlag::SockNonEmpty => self.sock_nonempty().is_some(),
            CqeFlag::Notification => self.notification().is_some(),
            CqeFlag::BufMore => self.buf_more().is_some(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.count_zeros() == u32::BITS
    }
}

impl From<u32> for CqeFlags {
    fn from(flags: u32) -> Self {
        Self(flags)
    }
}

//Same as an io_uring_cqe just without the user_data field
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrippedCqe {
    pub res: i32,
    pub flags: CqeFlags,
}

impl From<&io_uring_cqe> for StrippedCqe {
    fn from(cqe: &io_uring_cqe) -> Self {
        Self {
            res: cqe.res,
            flags: CqeFlags::from(cqe.flags),
        }
    }
}
