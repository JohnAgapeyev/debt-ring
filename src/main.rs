use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::str::FromStr;

use futures::AsyncWriteExt;
use nix::sys::socket::{AddressFamily, SockProtocol, SockType, SockaddrIn, SockaddrLike};

use clap::Parser;

use liburing_sys::*;

mod cqe;
mod executor;
mod handle;
mod ring;
mod sqe;
mod task;
mod tcp;

use crate::executor::*;
use crate::handle::*;
use crate::sqe::*;
use crate::tcp::*;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    listen: bool,
    host: String,
    #[arg(value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

//fn client(host: String, port: u16) {
//    unsafe {
//        let mut ring = Ring::new(32, 0).unwrap();
//
//        let connect_sqe = ring.get_sqe();
//
//        let sock = socket(
//            AddressFamily::Inet,
//            SockType::Stream,
//            SockFlag::SOCK_NONBLOCK,
//            SockProtocol::Tcp,
//        )
//        .unwrap();
//
//        let addr = SockaddrIn::from_str(&format!("{}:{}", host, port)).unwrap();
//
//        io_uring_prep_connect(
//            connect_sqe,
//            sock.as_raw_fd(),
//            addr.as_ptr() as *const liburing_sys::sockaddr,
//            addr.len(),
//        );
//
//        ring.submit().unwrap();
//
//        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//        assert!(res == 0);
//
//        let mut count = 0usize;
//
//        let mut buf = [0u8; 4096];
//
//        loop {
//            let now = Instant::now();
//            count += 1;
//
//            let send_sqe = ring.get_sqe();
//
//            let msg = "Hello io_uring world!\n";
//
//            io_uring_prep_send(
//                send_sqe,
//                sock.as_raw_fd(),
//                msg.as_bytes().as_ptr() as *const c_void,
//                msg.len(),
//                0,
//            );
//
//            ring.submit().unwrap();
//
//            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//            assert!(res == 0);
//
//            let raw = io_uring_cqe_get_data(cqe);
//            assert!(!raw.is_null());
//
//            let recv_sqe = ring.get_sqe();
//
//            let msg = "Hello io_uring world!\n";
//
//            io_uring_prep_recv(
//                recv_sqe,
//                sock.as_raw_fd(),
//                buf.as_mut_ptr() as *mut c_void,
//                msg.len(),
//                0,
//            );
//
//            ring.submit().unwrap();
//
//            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//            assert!(res == 0);
//
//            let raw = io_uring_cqe_get_data(cqe);
//            assert!(!raw.is_null());
//
//            println!(
//                "Time taken was: {} for message {}",
//                now.elapsed().as_nanos(),
//                count
//            );
//        }
//    };
//}
//
//fn server(host: String, port: u16) {
//    unsafe {
//        let mut ring = Ring::new(32, 0).unwrap();
//
//        let bind_sqe = ring.get_sqe();
//
//        let sock = socket(
//            AddressFamily::Inet,
//            SockType::Stream,
//            SockFlag::SOCK_NONBLOCK,
//            SockProtocol::Tcp,
//        )
//        .unwrap();
//
//        let addr = SockaddrIn::from_str(&format!("{}:{}", host, port)).unwrap();
//
//        io_uring_prep_bind(
//            bind_sqe,
//            sock.as_raw_fd(),
//            addr.as_ptr() as *mut liburing_sys::sockaddr,
//            addr.len(),
//        );
//
//        ring.submit().unwrap();
//
//        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//        assert!(res == 0);
//
//        io_uring_cqe_seen(&mut ring.inner, cqe);
//
//        let listen_sqe = ring.get_sqe();
//
//        io_uring_prep_listen(listen_sqe, sock.as_raw_fd(), 32);
//
//        ring.submit().unwrap();
//
//        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//        assert!(res == 0);
//
//        io_uring_cqe_seen(&mut ring.inner, cqe);
//
//        let accept_sqe = ring.get_sqe();
//
//        io_uring_prep_accept(
//            accept_sqe,
//            sock.as_raw_fd(),
//            std::ptr::null_mut(),
//            std::ptr::null_mut(),
//            SockFlag::SOCK_NONBLOCK.bits(),
//        );
//
//        ring.submit().unwrap();
//
//        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//        let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//        assert!(res == 0);
//        assert!(!cqe.is_null());
//        assert!((*cqe).res > 0);
//
//        println!("Resulting socket fd is: {}", (*cqe).res);
//
//        let accepted_sock = (*cqe).res;
//
//        io_uring_cqe_seen(&mut ring.inner, cqe);
//
//        let mut buf = [0u8; 4096];
//
//        let mut count = 0usize;
//
//        loop {
//            let now = Instant::now();
//            count += 1;
//
//            let send_sqe = ring.get_sqe();
//
//            let msg = "Hello io_uring world!\n";
//
//            io_uring_prep_send(
//                send_sqe,
//                accepted_sock,
//                msg.as_bytes().as_ptr() as *const c_void,
//                msg.len(),
//                0,
//            );
//
//            ring.submit().unwrap();
//
//            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//            assert!(res == 0);
//
//            let raw = io_uring_cqe_get_data(cqe);
//            assert!(!raw.is_null());
//
//            let recv_sqe = ring.get_sqe();
//
//            let msg = "Hello io_uring world!\n";
//
//            io_uring_prep_recv(
//                recv_sqe,
//                sock.as_raw_fd(),
//                buf.as_mut_ptr() as *mut c_void,
//                msg.len(),
//                0,
//            );
//
//            ring.submit().unwrap();
//
//            let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();
//
//            let res = io_uring_wait_cqe(&mut ring.inner, &mut cqe);
//            assert!(res == 0);
//
//            let raw = io_uring_cqe_get_data(cqe);
//            assert!(!raw.is_null());
//
//            println!(
//                "Time taken was: {} for message {}",
//                now.elapsed().as_nanos(),
//                count
//            );
//        }
//    };
//}

fn main() {
    spawn(async move {
        println!("I am an async function!");

        let nop_result = SqeFuture::nop().await;
        println!("CQE result: {nop_result:#?}");

        let socket_result = SqeFuture::socket(
            AddressFamily::Inet as i32,
            SockType::Stream as i32,
            SockProtocol::Tcp as i32,
            0,
        )
        .await;
        println!("CQE result: {socket_result:#?}");

        let owned_sock = socket_result.unwrap();

        let host = "127.0.0.1";
        let port = 8080;
        let addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();

        let connect_result = SqeFuture::connect(&owned_sock, addr).await;
        println!("Connect CQE result: {connect_result:#?}");

        let msg = "Hello io_uring world!\n";

        let send_result = SqeFuture::send(&owned_sock, msg.as_bytes(), 0).await;
        println!("Send CQE result: {send_result:#?}");

        let mut tcp = TcpStream::new().await;
        tcp.connect(addr).await.unwrap();
        let write_result = tcp.write(msg.as_bytes()).await;
        println!("TCP Write result: {write_result:#?}");

        let listen_host = "127.0.0.1";
        let listen_port = 8081;
        let listen_addr = SocketAddr::from_str(&format!("{}:{}", listen_host, listen_port)).unwrap();

        let listener = TcpListener::bind(listen_addr).await.unwrap();
        let (new_tcp, _) = listener.accept().await.unwrap();
        let write_result = new_tcp.write(msg.as_bytes()).await;
        println!("TCP Accepted Write result: {write_result:#?}");

        let mut buf = [0u8; 4096];
        let read_result = new_tcp.read(&mut buf).await.unwrap();
        println!("TCP Accepted Read result: {read_result:#?}");
        println!("TCP Accepted Read contents: {:#?}", str::from_utf8(&buf[0..read_result]).unwrap());
    });

    Handle::current().with_exec(|exec| {
        exec.run();
    });

    return;

    let cli = Cli::parse();

    //if cli.listen {
    //    server(cli.host, cli.port);
    //} else {
    //    client(cli.host, cli.port);
    //}
}
