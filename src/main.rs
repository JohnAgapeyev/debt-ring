use std::ffi::c_void;
use std::io::Error;

use liburing_sys::*;

struct Ring {
    pub inner: io_uring,
}

impl Ring {
    pub fn new(entries: u32, flags: u32) -> Result<Ring, Error> {
        unsafe {
            let mut ring: io_uring = std::mem::zeroed();
            match io_uring_queue_init(entries, &mut ring, flags) {
                0 => Ok(Ring { inner: ring }),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe { io_uring_queue_exit(&mut self.inner) }
    }
}

pub fn get_sqe(ring: &mut io_uring) -> Option<&mut io_uring_sqe> {
    unsafe { io_uring_get_sqe(ring).as_mut() }
}

fn main() {
    unsafe {
        let mut ring = Ring::new(32, 0).unwrap();

        let mut magic: i32 = 17;

        let sqe = get_sqe(&mut ring.inner).unwrap();

        io_uring_sqe_set_data(sqe, &mut magic as *mut _ as *mut c_void);

        io_uring_prep_nop(sqe);

        let submitted = io_uring_submit(&mut ring.inner);
        println!("Submitted {submitted} entries");

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        println!("Wait CQE result was: {res}");

        assert!(res == 0);

        let raw = io_uring_cqe_get_data(cqe);
        assert!(!raw.is_null());

        println!("My magic value is: {}", *(raw as *mut i32));
    };
    println!("Hello, world!");
}
