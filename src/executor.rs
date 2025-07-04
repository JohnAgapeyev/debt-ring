use futures::FutureExt;
use futures::future::LocalBoxFuture;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::io::Error;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Waker};
use std::time::Duration;

use crate::handle::Handle;
use crate::ring::Ring;
use crate::task::*;

pub(crate) struct Executor {
    ring: Rc<RefCell<Ring>>,
    future_map: RefCell<HashMap<u64, LocalBoxFuture<'static, ()>>>,
    task_queue: RefCell<VecDeque<u64>>,
}

impl Executor {
    pub fn new(ring: Rc<RefCell<Ring>>, entries: u32) -> Result<Self, Error> {
        let future_map = RefCell::new(HashMap::with_capacity(entries as usize));
        let task_queue = RefCell::new(VecDeque::with_capacity(entries as usize));
        Ok(Executor {
            ring,
            future_map,
            task_queue,
        })
    }
    pub fn wake(&self, task: Task) {
        self.task_queue.borrow_mut().push_back(task.into_id());
    }
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        let future = future.boxed_local();
        let task = get_next_task_id();
        self.future_map.borrow_mut().insert(task.into_id(), future);
        self.task_queue.borrow_mut().push_back(task.into_id());
    }
    fn should_continue_running(&self) -> bool {
        //This function may change in the future, but for now is a good heuristic
        self.future_map.borrow().is_empty() && self.task_queue.borrow().is_empty()
    }
    pub fn run(&self) {
        loop {
            if self.should_continue_running() {
                return;
            }

            while !self.task_queue.borrow().is_empty() {
                let front_entry = self.task_queue.borrow_mut().pop_front();
                if front_entry.is_none() {
                    break;
                }
                match front_entry {
                    None => break,
                    Some(task_id) => {
                        let mut future =
                            self.future_map.borrow_mut().remove(&task_id).expect(
                                "Task queue contained an ID not in the future map: {task_id}",
                            );

                        let waker = Waker::from(Arc::new(Task::new(task_id)));
                        let context = &mut Context::from_waker(&waker);
                        if future.as_mut().poll(context).is_pending() {
                            self.future_map.borrow_mut().insert(task_id, future);
                        }
                    }
                };
            }
            //No work in the queue to be done
            match self
                .ring
                .borrow()
                .submit_and_wait_timeout(Some(Duration::new(1, 0)))
            {
                Ok(_) => (),
                Err(e) => {
                    let inner = e.raw_os_error().unwrap();
                    //62 is ETIME errno value
                    if inner == 62 {
                        continue;
                    }
                    panic!("Got an unknown error: {}", inner);
                }
            };
        }
    }
}

pub fn spawn(future: impl Future<Output = ()> + 'static) {
    Handle::current().with_exec(|exec| {
        exec.spawn(future);
    })
}
