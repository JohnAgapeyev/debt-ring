use std::cell::RefCell;
use std::rc::Rc;

use crate::executor::Executor;
use crate::ring::Ring;

use liburing_sys::*;

const RING_FLAGS: u32 = IORING_SETUP_SINGLE_ISSUER
    | IORING_SETUP_COOP_TASKRUN
    | IORING_SETUP_DEFER_TASKRUN
    | IORING_SETUP_SUBMIT_ALL;

thread_local! {
    static RING: Rc<RefCell<Ring>> = Rc::new(RefCell::new(Ring::new(4096, RING_FLAGS).unwrap()));
    static EXECUTOR: Rc<RefCell<Executor>> = Rc::new(RefCell::new(Executor::new(RING.with(|ring| Rc::clone(ring)), 4096).unwrap()));
}

#[derive(Clone)]
pub(crate) struct Handle {
    inner: Rc<RefCell<Executor>>,
}

impl Handle {
    pub fn current() -> Self {
        EXECUTOR.with(|exec| Self {
            inner: Rc::clone(exec),
        })
    }
    pub fn with_exec<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Executor) -> R,
    {
        f(&self.inner.borrow())
    }
}

#[derive(Clone)]
pub(crate) struct RingHandle {
    inner: Rc<RefCell<Ring>>,
}

impl RingHandle {
    pub fn current() -> Self {
        RING.with(|ring| Self {
            inner: Rc::clone(ring),
        })
    }
    pub fn with_ring<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Ring) -> R,
    {
        f(&self.inner.borrow())
    }
}
