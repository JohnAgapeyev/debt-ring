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
use crate::sqe::SqeFutureShared;
use crate::task::*;

use liburing_sys::*;

pub(crate) struct Executor {
    ring: Ring,

    task_map: RefCell<HashMap<u64, Rc<RefCell<SqeFutureShared>>>>,
    future_map: RefCell<HashMap<u64, LocalBoxFuture<'static, ()>>>,

    task_queue: RefCell<VecDeque<u64>>,
}

impl Executor {
    pub fn new(entries: u32, flags: u32) -> Result<Self, Error> {
        let ring = Ring::new(entries, flags)?;
        let task_map = RefCell::new(HashMap::with_capacity(entries as usize));
        let future_map = RefCell::new(HashMap::with_capacity(entries as usize));
        let task_queue = RefCell::new(VecDeque::with_capacity(entries as usize));
        Ok(Executor {
            ring,
            task_map,
            future_map,
            task_queue,
        })
    }
    pub fn register_task(&self, task: Task, sqe: Rc<RefCell<SqeFutureShared>>) {
        self.task_map.borrow_mut().insert(task.into_id(), sqe);
    }
    pub fn wake(&self, task: Task) {
        self.task_queue.borrow_mut().push_back(task.into_id());
    }
    pub fn get_sqe(&self) -> &mut io_uring_sqe {
        self.ring.get_sqe()
    }
    pub fn submit(&self) -> Result<i32, Error> {
        self.ring.submit()
    }
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        let future = future.boxed_local();

        let task = get_next_task_id();
        self.future_map.borrow_mut().insert(task.into_id(), future);
        self.task_queue.borrow_mut().push_back(task.into_id());
    }
    fn should_continue_running(&self) -> bool {
        //This function may change in the future, but for now is a good heuristic
        self.task_map.borrow().is_empty()
            && self.future_map.borrow().is_empty()
            && self.task_queue.borrow().is_empty()
    }
    pub fn run(&self) {
        loop {
            if self.should_continue_running() {
                println!("No more work available for the runtime");
                return;
            }

            let mut map = self.future_map.borrow_mut();
            while let Some(task_id) = self.task_queue.borrow_mut().pop_front() {
                let future = map
                    .get_mut(&task_id)
                    .expect("Task queue contained an ID not in the future map");

                let waker = Waker::from(Arc::new(Task::new(task_id)));
                let context = &mut Context::from_waker(&waker);
                if future.as_mut().poll(context).is_ready() {
                    map.remove(&task_id);
                }
            }
            //No work in the queue to be done
            println!("No work in the queue!");

            match self
                .ring
                .submit_and_wait_timeout(Some(Duration::new(1, 0)), |task_id, cqe| {
                    //Got a CQE to process
                    println!("Got a CQE: {:#?}", cqe);
                    println!("Waking task: {:#?}", task_id);

                    let task = self
                        .task_map
                        .borrow_mut()
                        .remove(&task_id)
                        .expect("CQE user_data doesn't exist in the task map!");

                    let mut task_binding = task.borrow_mut();
                    task_binding.completed = true;
                    task_binding.cqe = Some(*cqe);
                    task_binding
                        .waker
                        .as_ref()
                        .expect("Got a completed task with no waker!")
                        .wake_by_ref();

                    println!("Done with task: {:#?}", task_id);
                }) {
                Ok(_) => (),
                Err(e) => {
                    let inner = e.raw_os_error().unwrap();
                    //62 is ETIME errno value
                    if inner == 62 {
                        println!("No more CQEs");
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
