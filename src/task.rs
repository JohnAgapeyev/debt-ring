use std::cell::Cell;
use std::sync::Arc;
use std::task::Wake;

use crate::handle::Handle;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Task(u64);

impl Task {
    pub fn new(id: u64) -> Task {
        Task(id)
    }
    pub fn into_id(self) -> u64 {
        self.0
    }
}

impl Wake for Task {
    fn wake(self: Arc<Self>) {
        Handle::current().with_exec(|exec| {
            exec.wake(*self);
        })
    }
}

pub fn get_next_task_id() -> Task {
    thread_local! {
        static NEXT_TASK_ID: Cell<u64> = const { Cell::new(1234u64) };
    }
    let out_id = NEXT_TASK_ID.get();
    NEXT_TASK_ID.set(out_id.wrapping_add(1));
    Task(out_id)
}
