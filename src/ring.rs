use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Error;
use std::rc::Rc;
use std::time::Duration;

use liburing_sys::*;

use crate::cqe::StrippedCqe;
use crate::sqe::SqeFutureShared;
use crate::task::*;

#[derive(Debug, Clone)]
pub struct Ring {
    inner: RefCell<io_uring>,
    cq_buf: RefCell<Vec<*mut io_uring_cqe>>,
    sqe_map: RefCell<HashMap<u64, Rc<RefCell<SqeFutureShared>>>>,
}

impl Ring {
    pub fn new(entries: u32, flags: u32) -> Result<Self, Error> {
        let cq_buf = RefCell::new(Vec::with_capacity(entries as usize));
        let sqe_map = RefCell::new(HashMap::with_capacity(entries as usize));
        unsafe {
            let mut ring: io_uring = std::mem::zeroed();
            let mut params: io_uring_params = std::mem::zeroed();
            params.flags = flags;
            match io_uring_queue_init_params(entries, &mut ring, &mut params) {
                0 => {
                    /*
                     * IORING_FEAT_SUBMIT_STABLE allows the key assumption that memory only
                     * has to survive until submit. Without it, the lifetimes get much more
                     * complicated to manage.
                     * 5.10 LTS is EOL as of December 2026, and I'm not going to finish the
                     * bulk of the work required here before then, so no point supporting
                     * something so complicated that won't matter for anything I will ever
                     * need to do
                     *
                     * man io_uring_queue_init(3):
                     *
                     * If this flag is set, applications can be certain that any data
                     * for async offload has been consumed when the kernel has consumed the SQE.
                     * Available since kernel 5.5.
                     *
                     * man io_uring_submit(3):
                     *
                     * For any request that passes in data in a struct,
                     * that data must remain valid until the request has been successfully submitted.
                     * It need not remain valid until completion.
                     * Once a request has been submitted, the in-kernel state is stable.
                     * Very early kernels (5.4 and earlier) required state to be stable until the completion occurred.
                     * Applications can test for this behavior by inspecting
                     * the IORING_FEAT_SUBMIT_STABLE flag passed back from io_uring_queue_init_params(3).
                     */
                    if (params.features & IORING_FEAT_SUBMIT_STABLE) == 0 {
                        panic!("Kernel doesn't support IORING_FEAT_SUBMIT_STABLE!");
                    }
                    Ok(Ring {
                        inner: RefCell::new(ring),
                        cq_buf,
                        sqe_map,
                    })
                }
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn register_task(&self, task: Task, sqe: Rc<RefCell<SqeFutureShared>>) {
        self.sqe_map.borrow_mut().insert(task.into_id(), sqe);
    }
    pub fn get_sqe(&self) -> &mut io_uring_sqe {
        unsafe {
            match io_uring_get_sqe(self.inner.as_ptr()).as_mut() {
                Some(sqe) => {
                    let task_id = get_next_task_id().into_id();
                    io_uring_sqe_set_data64(sqe, task_id);
                    sqe
                }
                None => {
                    self.submit()
                        .expect("io_uring_submit() failed after NULL SQE");
                    self.get_sqe()
                }
            }
        }
    }
    pub fn submit(&self) -> Result<i32, Error> {
        /*
         * If SQPOLL is used, the return value may report a higher number of submitted entries
         * than actually submitted. If the user requires accurate information about how many
         * submission queue entries have been successfully submitted, while using SQPOLL,
         * the user must fall back to repeatedly submitting a single submission queue entry.
         */
        unsafe {
            match io_uring_submit(self.inner.as_ptr()) {
                n if n >= 0 => Ok(n),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn submit_and_wait_timeout(&self, timeout: Option<Duration>) -> Result<i32, Error> {
        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let mut local_ts = __kernel_timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

        let ts = match timeout {
            Some(d) => {
                local_ts.tv_sec = d.as_secs() as i64;
                local_ts.tv_nsec = d.subsec_nanos() as i64;
                &mut local_ts
            }
            None => std::ptr::null_mut(),
        };

        let res = unsafe {
            io_uring_submit_and_wait_timeout(
                self.inner.as_ptr(),
                &mut cqe,
                1,
                ts,
                std::ptr::null_mut(),
            )
        };
        // On failure it returns -errno.
        if res < 0 {
            return Err(Error::from_raw_os_error(-res));
        }
        assert!(res >= 0);

        let mut cq_buf = self.cq_buf.borrow_mut();
        assert!(cq_buf.is_empty());
        let capacity = cq_buf.capacity();
        cq_buf.resize_with(capacity, std::ptr::null_mut);

        unsafe {
            let nfilled = io_uring_peek_batch_cqe(
                self.inner.as_ptr(),
                cq_buf.as_mut_ptr(),
                cq_buf.len() as u32,
            );
            if nfilled >= 1 {
                //Got some CQEs to process
                assert!(nfilled <= cq_buf.len() as u32);
                for i in 0..nfilled {
                    let cqe = cq_buf[i as usize];
                    assert!(!cqe.is_null());
                    let task_id = io_uring_cqe_get_data64(cqe);
                    let stripped = StrippedCqe::from(&*cqe);

                    //Got a CQE to process
                    let task = self
                        .sqe_map
                        .borrow_mut()
                        .remove(&task_id)
                        .expect("CQE user_data doesn't exist in the SQE map!");

                    let mut task_binding = task.borrow_mut();
                    task_binding.cqe = Some(stripped);
                    task_binding
                        .waker
                        .as_ref()
                        .expect("Got a completed task with no waker!")
                        .wake_by_ref();
                }
                io_uring_cq_advance(self.inner.as_ptr(), nfilled);
            }
        }
        cq_buf.clear();

        Ok(res)
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe { io_uring_queue_exit(self.inner.as_ptr()) }
    }
}
