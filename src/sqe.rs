use std::cell::RefCell;
use std::ffi::c_void;
use std::future::Future;
use std::io::Error;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::cqe::StrippedCqe;
use crate::handle::Handle;
use crate::task::*;

use liburing_sys::*;
use nix::sys::socket::{
    AddressFamily, SockProtocol, SockType, SockaddrIn, SockaddrLike, SockaddrStorage,
};

#[derive(Debug, Clone)]
#[must_use = "Futures do nothing if not awaited"]
pub struct SqeFuture {
    pub shared: Rc<RefCell<SqeFutureShared>>,
}

#[derive(Debug, Clone)]
pub struct SqeFutureShared {
    pub waker: Option<Waker>,
    pub cqe: Option<StrippedCqe>,
}

/*
 * SQE operation requirements:
 *  - Needs a reference to the Ring to be able to be fetched
 *  - Once fetched, it is considered "alive" as far as submitting events go
 *  - Seems like default zeroed version is same as the no-op, but don't want to rely on that
 *  - You call an io_uring_prep_* function to set up the SQE per-operation
 *  - You only want to call the prep function once on a single SQE
 *  - I want a single interface for handling the N different possible prep/operations here
 *  - I want to minimize boilerplate of making N newtypes to boot
 *  - The io_uring_prep_* functions have different args, so traits don't work nicely
 *  - I don't want to allow SQEs to be created without calling a prep function on them
 */

impl Future for SqeFuture {
    type Output = StrippedCqe;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.shared.borrow_mut();
        if shared.cqe.is_none() {
            if let Some(wake) = &mut shared.waker {
                wake.clone_from(cx.waker());
            } else {
                shared.waker = Some(cx.waker().clone());
            }
            return Poll::Pending;
        }
        Poll::Ready(
            shared
                .cqe
                .expect("Future was ready without a CQE result stored!"),
        )
    }
}

impl SqeFuture {
    fn new() -> SqeFuture {
        SqeFuture {
            shared: Rc::new(RefCell::new(SqeFutureShared {
                waker: None,
                cqe: None,
            })),
        }
    }
    fn prep_and_register<F>(prep_fn: F) -> Self
    where
        F: Fn(&mut io_uring_sqe),
    {
        Handle::current().with_exec(|exec| {
            let fut = SqeFuture::new();
            let sqe = exec.get_sqe();
            prep_fn(sqe);
            exec.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
            fut
        })
    }

    pub async fn nop() -> Result<(), Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_nop(sqe);
        })
        .await;
        if cqe.res != 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.flags.is_empty());
        Ok(())
    }
    pub async fn socket(
        domain: i32,
        sock_type: i32,
        protocol: i32,
        flags: u32,
    ) -> Result<OwnedFd, Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_socket(sqe, domain, sock_type, protocol, flags);
        })
        .await;
        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.flags.is_empty());
        let fd = unsafe { OwnedFd::from_raw_fd(cqe.res) };
        Ok(fd)
    }
    pub async fn connect(sockfd: &impl AsRawFd, addr: SocketAddr) -> Result<(), Error> {
        let addr_storage = SockaddrStorage::from(addr);

        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_connect(
                sqe,
                sockfd.as_raw_fd(),
                addr_storage.as_ptr() as *const liburing_sys::sockaddr,
                SockaddrStorage::size(),
            );
        })
        .await;
        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.res == 0);
        assert!(cqe.flags.is_empty());
        Ok(())
    }
    pub async fn send(sockfd: &impl AsRawFd, buf: &[u8], flags: i32) -> Result<usize, Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_send(
                sqe,
                sockfd.as_raw_fd(),
                buf.as_ptr() as *const c_void,
                buf.len(),
                flags,
            );
        })
        .await;

        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.flags.is_empty());
        Ok(cqe.res.try_into().unwrap())
    }
    pub fn close(sockfd: impl AsRawFd) -> Self {
        Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_close(sqe, sockfd.as_raw_fd());
        })
    }
}
