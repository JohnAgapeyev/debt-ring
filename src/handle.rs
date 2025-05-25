use std::cell::RefCell;
use std::rc::Rc;

use crate::executor::Executor;

use liburing_sys::*;

thread_local! {
    static EXECUTOR: Rc<RefCell<Executor>> = Rc::new(RefCell::new(Executor::new(4096, IORING_SETUP_SINGLE_ISSUER | IORING_SETUP_COOP_TASKRUN | IORING_SETUP_DEFER_TASKRUN | IORING_SETUP_SUBMIT_ALL).unwrap()));
}

#[derive(Clone)]
pub(crate) struct Handle {
    inner: Rc<RefCell<Executor>>,
}

impl Handle {
    pub fn current() -> Handle {
        EXECUTOR.with(|exec| Handle {
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
