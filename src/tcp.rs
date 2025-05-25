use std::io::Error;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd};
use std::pin::{Pin, pin};
use std::task::{Context, Poll, ready};

use crate::sqe::SqeFuture;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use liburing_sys::SO_REUSEADDR;
use liburing_sys::SOL_SOCKET;
use nix::sys::socket::{AddressFamily, SockProtocol, SockType, SockaddrIn, SockaddrLike};
use std::ffi::c_void;

/*
 * I don't like the current situation with handling state around the AsyncRead/AsyncWrite trait
 * implementation requirements with polling
 * Fundamentally, io_uring is an ownership based system because it focuses on completions
 * The tactics used by futures and tokio don't work here because of the ownership need.
 * So when you try to write a buffer, tokio will poll for readiness and then do the call directly.
 * Doing AsyncWriteExt calls on a type returns a trivial wrapper that implements Future simply
 * by delegating the poll to the poll_write of the underlying AsyncWrite type
 *
 * This becomes quite a struggle around spurious wakeup polling, since in a stateless reference
 * world like futures/tokio, you can simply redo the same operations with no change.
 * However, on the io_uring side, you've already submitted the operation and need to wait for
 * completion.
 *
 * So how do you "know" what operation to poll in the delegation to?
 * You need some state object to reference to poll
 * But you can't store that state inside the object/future itself due to reference sharing race
 * conditions.
 * Say for example you are implenting a UDP socket.
 * It's totally valid to use a Rc<OwnedFd> and share it between async tasks.
 * The kernel syscalls are atomic and well-defined, and the SQE fetching is as well.
 * The problem is on the state handling.
 * If you store the state inside the UDP socket type, what happens on a spurious wakeup?
 * You now have two valid instances of write that requires state that needs polling.
 * Which obviously generalizes up to N, requiring dynamic size lists here.
 * But even if you make a Vec<Future> for yourself, now you need to know which one to poll!
 *
 * What do you have to identify the specific operation?
 * In the case of write, it's the buffer slice, but now you're doing a full memcmp just to re-poll?
 * The entire mess gets very silly very quickly.
 *
 * I'm already going to have to go out into the unknown when trying to do fancier things like
 * linking SQEs together and juggling registered files/buffers.
 * So perhaps the pain of trying to stay compatible with legacy community traits is holding me back
 * here?
 * I need ownership, make a trait with ownership.
 * The whole reason these wrapper types and AsyncWrite's poll_write() function exist in the first
 * place is due to Rust language limitations with async and traits!
 * We now have async traits since 1.75, with some big limitations, but still, there's no need to
 * juggle all of this chaos for compatibility that is going to be knowingly suboptimal in the first
 * place!
 *
 * Better to do your own thing and borrow naming and conventions rather than trying to explicitly
 * remain compatible with that part of the ecosystem.
 * The whole reason I'm building this project is to avoid the traditional io_uring integrations
 * that are done by libuv/ASIO and others that still end up wrapping epoll under the hood.
 * It's not just a different faster poll(), it's closer to IOCP, and should be treated as such.
 */

pub struct TcpStream {
    sock: OwnedFd,
}

impl TcpStream {
    pub(crate) fn from_raw(sock: OwnedFd) -> Self {
        Self { sock }
    }

    pub async fn new() -> Self {
        let sock = SqeFuture::socket(
            AddressFamily::Inet as i32,
            SockType::Stream as i32,
            SockProtocol::Tcp as i32,
            0,
        )
        .await
        .unwrap();

        Self { sock }
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<(), Error> {
        SqeFuture::connect(&self.sock, addr).await
    }
    pub async fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        SqeFuture::send(&self.sock, buf, 0).await
    }
    pub async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        SqeFuture::recv(&self.sock, buf, 0).await
    }
}

pub struct TcpListener {
    sock: OwnedFd,
}

impl TcpListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self, Error> {
        let sock = SqeFuture::socket(
            AddressFamily::Inet as i32,
            SockType::Stream as i32,
            SockProtocol::Tcp as i32,
            0,
        )
        .await?;

        //tokio sets SO_REUSEADDR as part of the bind call
        let mut enable = 1;
        let flag_size = 4;

        SqeFuture::setsockopt(
            &sock,
            SOL_SOCKET.try_into().unwrap(),
            SO_REUSEADDR.try_into().unwrap(),
            &mut enable as *mut _ as *mut c_void,
            flag_size,
        )
        .await?;

        SqeFuture::bind(&sock, addr).await?;
        SqeFuture::listen(&sock, 32).await?;

        Ok(TcpListener { sock })
    }

    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr), Error> {
        let (fd, addr) = SqeFuture::accept(&self.sock, 0).await?;

        let inet_addr = addr.as_sockaddr_in().unwrap();
        let std_addr = SocketAddrV4::new(inet_addr.ip(), inet_addr.port());

        Ok((TcpStream::from_raw(fd), SocketAddr::from(std_addr)))
    }
}
