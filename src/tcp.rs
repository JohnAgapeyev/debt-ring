use std::io::Error;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, OwnedFd};
use std::pin::{Pin, pin};
use std::task::{Context, Poll, ready};

use crate::sqe::SqeFuture;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use futures::io::AsyncWrite;
use nix::sys::socket::{AddressFamily, SockProtocol, SockType, SockaddrIn, SockaddrLike};
use pin_project::pin_project;


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

#[pin_project]
pub struct TcpStream {
    sock: OwnedFd,
    //write_fut: Option<Pin<Box<dyn Future<Output = Result<usize, Error>>>>>,
    #[pin]
    write_fut: Option<Pin<Box<SqeFuture>>>,
    #[pin]
    write_buf: Option<Box<[u8]>>,
    #[pin]
    close_fut: Option<Pin<Box<SqeFuture>>>,
}

impl TcpStream {
    pub async fn new() -> Self {
        let sock = SqeFuture::socket(
            AddressFamily::Inet as i32,
            SockType::Stream as i32,
            SockProtocol::Tcp as i32,
            0,
        )
        .await
        .unwrap();

        Self {
            sock,
            write_fut: None,
            write_buf: None,
            close_fut: None,
        }
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<(), Error> {
        SqeFuture::connect(&self.sock, addr).await
    }

    fn poll_write_priv(&mut self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, Error>> {
        if self.write_fut.is_none() {
            let fd = self.sock.as_raw_fd();
            self.write_buf = Some(Box::from(buf));
            self.write_fut = Some(Box::pin(SqeFuture::send(
                fd,
                self.write_buf.as_ref().unwrap().as_ref(),
                0,
            )));
        }

        let cqe = ready!(self.write_fut.as_mut().unwrap().as_mut().poll(cx));

        if cqe.res < 0 {
            return Poll::Ready(Err(Error::from_raw_os_error(-cqe.res)));
        }
        assert!(cqe.flags.is_empty());
        Poll::Ready(Ok(cqe.res.try_into().unwrap()))
    }
    fn poll_close_priv(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.close_fut.is_none() {
            let fd = self.sock.as_raw_fd();
            self.close_fut = Some(Box::pin(SqeFuture::close(fd)));
        }

        let cqe = ready!(self.close_fut.as_mut().unwrap().as_mut().poll(cx));

        if cqe.res != 0 {
            return Poll::Ready(Err(Error::from_raw_os_error(-cqe.res)));
        }
        assert!(cqe.flags.is_empty());
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        self.poll_write_priv(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.poll_close_priv(cx)
    }
}
