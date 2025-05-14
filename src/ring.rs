use std::cell::RefCell;
use std::io::Error;
use std::time::Duration;

use liburing_sys::*;

use crate::cqe::StrippedCqe;
use crate::task::*;

#[derive(Debug, Clone)]
pub struct Ring {
    pub inner: RefCell<io_uring>,
    pub cq_buf: RefCell<Vec<*mut io_uring_cqe>>,
}

impl Ring {
    pub fn new(entries: u32, flags: u32) -> Result<Self, Error> {
        let mut cq_buf = Vec::with_capacity(entries as usize);
        cq_buf.resize_with(entries as usize, std::ptr::null_mut);
        unsafe {
            let mut ring: io_uring = std::mem::zeroed();
            match io_uring_queue_init(entries, &mut ring, flags) {
                0 => Ok(Ring {
                    inner: RefCell::new(ring),
                    cq_buf: RefCell::new(cq_buf),
                }),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn get_sqe(&self) -> &mut io_uring_sqe {
        unsafe {
            match io_uring_get_sqe(self.inner.as_ptr()).as_mut() {
                Some(sqe) => {
                    let task_id = get_next_task_id().into_id();
                    println!("Creating SQE with ID: {task_id}");
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
    pub fn submit_and_wait_timeout<F>(
        &self,
        timeout: Option<Duration>,
        cqe_handler: F,
    ) -> Result<i32, Error>
    where
        F: Fn(u64, &StrippedCqe),
    {
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
        cq_buf.clear();
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
                    cqe_handler(task_id, &stripped);
                }
                io_uring_cq_advance(self.inner.as_ptr(), nfilled);
            }
        }

        Ok(res)
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe { io_uring_queue_exit(self.inner.as_ptr()) }
    }
}
