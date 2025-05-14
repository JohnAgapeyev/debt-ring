use std::cell::RefCell;
use std::rc::Rc;

use crate::executor::Executor;

thread_local! {
    static EXECUTOR: Rc<RefCell<Executor>> = Rc::new(RefCell::new(Executor::new(32, 0).unwrap()));
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
