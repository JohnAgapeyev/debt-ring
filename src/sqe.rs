use std::cell::RefCell;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::cqe::StrippedCqe;
use crate::handle::Handle;
use crate::task::*;

use liburing_sys::*;

pub struct SqeFuture {
    pub shared: Rc<RefCell<SqeFutureShared>>,
}

#[derive(Debug, Clone)]
pub struct SqeFutureShared {
    pub waker: Option<Waker>,
    pub cqe: Option<StrippedCqe>,
    pub completed: bool,
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
        if !shared.completed {
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
                completed: false,
            })),
        }
    }
    #[must_use]
    pub fn nop() -> SqeFuture {
        Handle::current().with_exec(|exec| {
            let fut = SqeFuture::new();
            let sqe = exec.get_sqe();
            unsafe {
                io_uring_prep_nop(sqe);
            }
            exec.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
            fut
        })
    }
    #[must_use]
    pub fn socket(domain: i32, sock_type: i32, protocol: i32, flags: u32) -> SqeFuture {
        Handle::current().with_exec(|exec| {
            let fut = SqeFuture::new();
            let sqe = exec.get_sqe();
            unsafe {
                io_uring_prep_socket(sqe, domain, sock_type, protocol, flags);
            }
            exec.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
            fut
        })
    }
    #[must_use]
    pub fn connect(sockfd: i32, addr: *const sockaddr, addrlen: socklen_t) -> SqeFuture {
        Handle::current().with_exec(|exec| {
            let fut = SqeFuture::new();
            let sqe = exec.get_sqe();
            unsafe {
                io_uring_prep_connect(sqe, sockfd, addr, addrlen);
            }
            exec.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
            fut
        })
    }
    #[must_use]
    pub fn send(sockfd: i32, buf: &[u8], flags: i32) -> SqeFuture {
        Handle::current().with_exec(|exec| {
            let fut = SqeFuture::new();
            let sqe = exec.get_sqe();
            unsafe {
                io_uring_prep_send(sqe, sockfd, buf.as_ptr() as *const c_void, buf.len(), flags);
            }
            exec.register_task(Task::new(sqe.user_data), Rc::clone(&fut.shared));
            fut
        })
    }
}
