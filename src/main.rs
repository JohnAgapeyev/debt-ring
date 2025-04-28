use liburing_sys::*;

fn main() {
    unsafe { io_uring_setup(32, std::ptr::null_mut()) };
    println!("Hello, world!");
}
