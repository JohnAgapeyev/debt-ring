use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::str::FromStr;
use std::time::Instant;

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

async fn client(host: String, port: u16) {
    let addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();

    let tcp = TcpStream::new().await;
    tcp.connect(addr).await.unwrap();

    let mut count = 0usize;

    let mut buf = [0u8; 4096];

    loop {
        let now = Instant::now();
        count += 1;

        let msg = "Hello io_uring world!\n";

        let write_result = tcp.write(msg.as_bytes()).await.unwrap();

        let read_result = tcp.read(&mut buf).await.unwrap();

        println!(
            "Time taken was: {} for message {}",
            now.elapsed().as_nanos(),
            count
        );
    }
}

async fn server(host: String, port: u16) {
    let listen_addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();

    let listener = TcpListener::bind(listen_addr).await.unwrap();
    let (new_tcp, _) = listener.accept().await.unwrap();

    let mut buf = [0u8; 4096];
    let mut count = 0usize;

    loop {
        let now = Instant::now();
        count += 1;

        let msg = "Hello io_uring world!\n";

        let write_result = new_tcp.write(msg.as_bytes()).await.unwrap();

        let read_result = new_tcp.read(&mut buf).await.unwrap();

        println!(
            "Time taken was: {} for message {}",
            now.elapsed().as_nanos(),
            count
        );
    }
}

fn main() {
    //spawn(async move {
    //    println!("I am an async function!");

    //    let nop_result = SqeFuture::nop().await;
    //    println!("CQE result: {nop_result:#?}");

    //    let socket_result = SqeFuture::socket(
    //        AddressFamily::Inet as i32,
    //        SockType::Stream as i32,
    //        SockProtocol::Tcp as i32,
    //        0,
    //    )
    //    .await;
    //    println!("CQE result: {socket_result:#?}");

    //    let owned_sock = socket_result.unwrap();

    //    let host = "127.0.0.1";
    //    let port = 8080;
    //    let addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();

    //    let connect_result = SqeFuture::connect(&owned_sock, addr).await;
    //    println!("Connect CQE result: {connect_result:#?}");

    //    let msg = "Hello io_uring world!\n";

    //    let send_result = SqeFuture::send(&owned_sock, msg.as_bytes(), 0).await;
    //    println!("Send CQE result: {send_result:#?}");

    //    let mut tcp = TcpStream::new().await;
    //    tcp.connect(addr).await.unwrap();
    //    let write_result = tcp.write(msg.as_bytes()).await;
    //    println!("TCP Write result: {write_result:#?}");

    //    let listen_host = "127.0.0.1";
    //    let listen_port = 8081;
    //    let listen_addr =
    //        SocketAddr::from_str(&format!("{}:{}", listen_host, listen_port)).unwrap();

    //    let listener = TcpListener::bind(listen_addr).await.unwrap();
    //    let (new_tcp, _) = listener.accept().await.unwrap();
    //    let write_result = new_tcp.write(msg.as_bytes()).await;
    //    println!("TCP Accepted Write result: {write_result:#?}");

    //    let mut buf = [0u8; 4096];
    //    let read_result = new_tcp.read(&mut buf).await.unwrap();
    //    println!("TCP Accepted Read result: {read_result:#?}");
    //    println!(
    //        "TCP Accepted Read contents: {:#?}",
    //        str::from_utf8(&buf[0..read_result]).unwrap()
    //    );
    //});

    let cli = Cli::parse();

    if cli.listen {
        spawn(server(cli.host, cli.port));
    } else {
        spawn(client(cli.host, cli.port));
    }

    Handle::current().with_exec(|exec| {
        exec.run();
    });
}
