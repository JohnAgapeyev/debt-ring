use liburing_sys::*;

fn main() {
    unsafe {
        let mut ring: io_uring = std::mem::zeroed();
        io_uring_queue_init(32, &mut ring, 0);

        let mut sqe = io_uring_get_sqe(&mut ring);

        io_uring_queue_exit(&mut ring);
    };
    println!("Hello, world!");
}
