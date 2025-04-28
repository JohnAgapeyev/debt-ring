use liburing_sys::*;

fn main() {
    unsafe {
        let mut params: io_uring_params = std::mem::zeroed();
        io_uring_setup(32, &mut params);
    };
    println!("Hello, world!");
}
