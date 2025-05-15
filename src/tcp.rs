use std::cell::RefCell;
use std::ffi::c_void;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::cqe::StrippedCqe;
use crate::handle::Handle;
use crate::sqe::SqeFuture;
use crate::task::*;

use nix::sys::socket::{AddressFamily, SockProtocol, SockType, SockaddrIn, SockaddrLike};

#[derive(Debug)]
pub struct TcpStream {
    sock: Option<OwnedFd>,
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

        Self { sock: Some(sock) }
    }
}
