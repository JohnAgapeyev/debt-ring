use std::ffi::c_void;
use std::io::Error;
use std::os::fd::AsRawFd;
use std::str::FromStr;
use std::time::Instant;

use nix::sys::socket::{
    AddressFamily, SockFlag, SockProtocol, SockType, SockaddrIn, SockaddrLike, socket,
};

use clap::Parser;

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

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    listen: bool,
    host: String,
    #[arg(value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

fn main() {
    let cli = Cli::parse();

    if cli.listen {
        println!("We don't currently support server listening mode!");
        return;
    }

    unsafe {
        let mut ring = Ring::new(32, 0).unwrap();

        let mut magic: i32 = 17;

        let now = Instant::now();

        let connect_sqe = get_sqe(&mut ring.inner).unwrap();

        io_uring_sqe_set_data(connect_sqe, &mut magic as *mut _ as *mut c_void);

        let sock = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            SockProtocol::Tcp,
        )
        .unwrap();

        let addr = SockaddrIn::from_str(&format!("{}:{}", cli.host, cli.port)).unwrap();

        io_uring_prep_connect(
            connect_sqe,
            sock.as_raw_fd(),
            addr.as_ptr() as *const liburing_sys::sockaddr,
            addr.len(),
        );

        let send_sqe = get_sqe(&mut ring.inner).unwrap();

        io_uring_sqe_set_data(send_sqe, &mut magic as *mut _ as *mut c_void);

        let msg = "Hello io_uring world!\n";

        io_uring_prep_send(
            send_sqe,
            sock.as_raw_fd(),
            msg.as_bytes().as_ptr() as *const c_void,
            msg.len(),
            0,
        );

        let submitted = io_uring_submit(&mut ring.inner);
        println!("Submitted {submitted} entries");

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        println!("Wait CQE result was: {res}");

        assert!(res == 0);

        let raw = io_uring_cqe_get_data(cqe);
        assert!(!raw.is_null());

        println!("Time taken was: {}", now.elapsed().as_micros());

        println!("My magic value is: {}", *(raw as *mut i32));
    };
    println!("Hello, world!");
}
