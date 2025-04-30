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
    pub cq_buf: Vec<*mut io_uring_cqe>,
}

impl Ring {
    pub fn new(entries: u32, flags: u32) -> Result<Ring, Error> {
        let cq_buf = Vec::with_capacity(entries as usize);
        unsafe {
            let mut ring: io_uring = std::mem::zeroed();
            match io_uring_queue_init(entries, &mut ring, flags) {
                0 => Ok(Ring {
                    inner: ring,
                    cq_buf,
                }),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn get_sqe<T>(&mut self, user_data: *mut T) -> Option<&mut io_uring_sqe> {
        unsafe {
            let sqe = io_uring_get_sqe(&mut self.inner).as_mut()?;
            io_uring_sqe_set_data(sqe, user_data as *mut c_void);
            Some(sqe)
        }
    }
    pub fn submit(&mut self) -> Result<i32, Error> {
        /*
         * If SQPOLL is used, the return value may report a higher number of submitted entries
         * than actually submitted. If the user requires accurate information about how many
         * submission queue entries have been successfully submitted, while using SQPOLL,
         * the user must fall back to repeatedly submitting a single submission queue entry.
         */
        unsafe {
            match io_uring_submit(&mut self.inner) {
                n if n >= 0 => Ok(n),
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

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    listen: bool,
    host: String,
    #[arg(value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

fn client(host: String, port: u16) {
    unsafe {
        let mut ring = Ring::new(32, 0).unwrap();

        let mut magic: i32 = 17;

        let connect_sqe = ring.get_sqe(&mut magic).unwrap();

        let sock = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            SockProtocol::Tcp,
        )
        .unwrap();

        let addr = SockaddrIn::from_str(&format!("{}:{}", host, port)).unwrap();

        io_uring_prep_connect(
            connect_sqe,
            sock.as_raw_fd(),
            addr.as_ptr() as *const liburing_sys::sockaddr,
            addr.len(),
        );

        ring.submit().unwrap();

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        assert!(res == 0);

        let mut count = 0usize;

        let mut buf = [0u8; 4096];

        loop {
            let now = Instant::now();
            count += 1;

            let send_sqe = ring.get_sqe(&mut magic).unwrap();

            let msg = "Hello io_uring world!\n";

            io_uring_prep_send(
                send_sqe,
                sock.as_raw_fd(),
                msg.as_bytes().as_ptr() as *const c_void,
                msg.len(),
                0,
            );

            ring.submit().unwrap();

            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
            assert!(res == 0);

            let raw = io_uring_cqe_get_data(cqe);
            assert!(!raw.is_null());

            let recv_sqe = ring.get_sqe(&mut magic).unwrap();

            let msg = "Hello io_uring world!\n";

            io_uring_prep_recv(
                recv_sqe,
                sock.as_raw_fd(),
                buf.as_mut_ptr() as *mut c_void,
                msg.len(),
                0,
            );

            ring.submit().unwrap();

            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
            assert!(res == 0);

            let raw = io_uring_cqe_get_data(cqe);
            assert!(!raw.is_null());

            println!(
                "Time taken was: {} for message {}",
                now.elapsed().as_nanos(),
                count
            );
        }
    };
}

fn server(host: String, port: u16) {
    unsafe {
        let mut ring = Ring::new(32, 0).unwrap();

        let mut magic: i32 = 17;

        let bind_sqe = ring.get_sqe(&mut magic).unwrap();

        let sock = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            SockProtocol::Tcp,
        )
        .unwrap();

        let addr = SockaddrIn::from_str(&format!("{}:{}", host, port)).unwrap();

        io_uring_prep_bind(
            bind_sqe,
            sock.as_raw_fd(),
            addr.as_ptr() as *mut liburing_sys::sockaddr,
            addr.len(),
        );

        ring.submit().unwrap();

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        assert!(res == 0);

        io_uring_cqe_seen(&mut ring.inner, cqe);

        let listen_sqe = ring.get_sqe(&mut magic).unwrap();

        io_uring_prep_listen(listen_sqe, sock.as_raw_fd(), 32);

        ring.submit().unwrap();

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        assert!(res == 0);

        io_uring_cqe_seen(&mut ring.inner, cqe);

        let accept_sqe = ring.get_sqe(&mut magic).unwrap();

        io_uring_prep_accept(
            accept_sqe,
            sock.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            SockFlag::SOCK_NONBLOCK.bits(),
        );

        ring.submit().unwrap();

        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
        assert!(res == 0);
        assert!(!cqe.is_null());
        assert!((*cqe).res > 0);

        println!("Resulting socket fd is: {}", (*cqe).res);

        let accepted_sock = (*cqe).res;

        io_uring_cqe_seen(&mut ring.inner, cqe);

        let mut buf = [0u8; 4096];

        let mut count = 0usize;

        loop {
            let now = Instant::now();
            count += 1;

            let send_sqe = ring.get_sqe(&mut magic).unwrap();

            let msg = "Hello io_uring world!\n";

            io_uring_prep_send(
                send_sqe,
                accepted_sock,
                msg.as_bytes().as_ptr() as *const c_void,
                msg.len(),
                0,
            );

            ring.submit().unwrap();

            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
            assert!(res == 0);

            let raw = io_uring_cqe_get_data(cqe);
            assert!(!raw.is_null());

            let recv_sqe = ring.get_sqe(&mut magic).unwrap();

            let msg = "Hello io_uring world!\n";

            io_uring_prep_recv(
                recv_sqe,
                sock.as_raw_fd(),
                buf.as_mut_ptr() as *mut c_void,
                msg.len(),
                0,
            );

            ring.submit().unwrap();

            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
            assert!(res == 0);

            let raw = io_uring_cqe_get_data(cqe);
            assert!(!raw.is_null());

            println!(
                "Time taken was: {} for message {}",
                now.elapsed().as_nanos(),
                count
            );
        }
    };
}

fn main() {
    let cli = Cli::parse();

    if cli.listen {
        server(cli.host, cli.port);
    } else {
        client(cli.host, cli.port);
    }
}
