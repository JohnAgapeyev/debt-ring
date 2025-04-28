use std::ffi::c_void;

use liburing_sys::*;

fn main() {
    unsafe {
        let mut ring: io_uring = std::mem::zeroed();
        io_uring_queue_init(32, &mut ring, 0);

        let mut magic: i32 = 17;

        let sqe = io_uring_get_sqe(&mut ring);

        io_uring_sqe_set_data(sqe, &mut magic as *mut _ as *mut c_void);

        io_uring_prep_nop(sqe);

        let submitted = io_uring_submit(&mut ring);
        println!("Submitted {submitted} entries");

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring, &mut cqe);
        println!("Wait CQE result was: {res}");

        assert!(res == 0);

        let raw = io_uring_cqe_get_data(cqe);
        assert!(!raw.is_null());

        println!("My magic value is: {}", *(raw as *mut i32));

        io_uring_queue_exit(&mut ring);
    };
    println!("Hello, world!");
}
