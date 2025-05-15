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
