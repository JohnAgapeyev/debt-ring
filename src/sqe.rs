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
use crate::handle::RingHandle;
use crate::ring::Ring;
use crate::task::*;

use liburing_sys::*;
use nix::sys::socket::{SockaddrLike, SockaddrStorage};

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
    pub(crate) fn new() -> Self {
        Self {
            shared: Rc::new(RefCell::new(SqeFutureShared {
                waker: None,
                cqe: None,
            })),
        }
    }
    pub(crate) fn prep_and_register<F>(prep_fn: F) -> Self
    where
        F: Fn(&mut io_uring_sqe),
    {
        RingHandle::current().with_ring(|ring| {
            let fut = SqeFuture::new();
            let sqe = ring.get_sqe();
            prep_fn(sqe);
            ring.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
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
    pub async fn bind(sockfd: &impl AsRawFd, addr: SocketAddr) -> Result<(), Error> {
        let addr_storage = SockaddrStorage::from(addr);

        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_bind(
                sqe,
                sockfd.as_raw_fd(),
                addr_storage.as_ptr() as *mut liburing_sys::sockaddr,
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
    pub async fn recv(sockfd: &impl AsRawFd, buf: &mut [u8], flags: i32) -> Result<usize, Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_recv(
                sqe,
                sockfd.as_raw_fd(),
                buf.as_ptr() as *mut c_void,
                buf.len(),
                flags,
            );
        })
        .await;

        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        //The SOCK_NONEMPTY flag may be set here, worth being aware of
        Ok(cqe.res.try_into().unwrap())
    }
    pub async fn close(sockfd: impl AsRawFd) -> Result<(), Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_close(sqe, sockfd.as_raw_fd());
        })
        .await;

        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.flags.is_empty());
        Ok(())
    }
    pub async fn listen(sockfd: &impl AsRawFd, backlog: i32) -> Result<(), Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_listen(sqe, sockfd.as_raw_fd(), backlog);
        })
        .await;

        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }
        assert!(cqe.res == 0);
        assert!(cqe.flags.is_empty());
        Ok(())
    }
    pub async fn accept(
        sockfd: &impl AsRawFd,
        flags: i32,
    ) -> Result<(OwnedFd, SockaddrStorage), Error> {
        let mut recv_addr: liburing_sys::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut recv_addr_size: u32 = std::mem::size_of::<liburing_sys::sockaddr_storage>()
            .try_into()
            .unwrap();

        let addr_ptr = std::ptr::addr_of_mut!(recv_addr) as *mut liburing_sys::sockaddr;
        let addr_size_ptr = std::ptr::addr_of_mut!(recv_addr_size);

        let cqe = Self::prep_and_register(|sqe| unsafe {
            io_uring_prep_accept(sqe, sockfd.as_raw_fd(), addr_ptr, addr_size_ptr, flags);
        })
        .await;

        if cqe.res < 0 {
            return Err(Error::from_raw_os_error(-cqe.res));
        }

        let fd = unsafe { OwnedFd::from_raw_fd(cqe.res) };
        let addr = unsafe {
            SockaddrStorage::from_raw(addr_ptr as *const nix::libc::sockaddr, Some(recv_addr_size))
        }
        .unwrap();
        Ok((fd, addr))
    }
    pub async fn setsockopt(
        sockfd: &impl AsRawFd,
        level: i32,
        optname: i32,
        optval: *mut c_void,
        optlen: i32,
    ) -> Result<(), Error> {
        let cqe = Self::prep_and_register(move |sqe| unsafe {
            io_uring_prep_cmd_sock(
                sqe,
                io_uring_socket_op_SOCKET_URING_OP_SETSOCKOPT
                    .try_into()
                    .unwrap(),
                sockfd.as_raw_fd(),
                level,
                optname,
                optval,
                optlen,
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
}
