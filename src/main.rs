use futures::FutureExt;
use futures::future::{BoxFuture, LocalBoxFuture};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use nix::sys::socket::{
    AddressFamily, SockFlag, SockProtocol, SockType, SockaddrIn, SockaddrLike, socket,
};
use nix::sys::time::TimeSpec;

use clap::Parser;

use liburing_sys::*;

thread_local! {
    static EXECUTOR: RefCell<Rc<RefCell<Executor>>> = RefCell::new(Rc::new(RefCell::new(Executor::new(32, 0).unwrap())));
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Task(u64);

impl Wake for Task {
    fn wake(self: Arc<Self>) {
        let cloned = self.clone();

        EXECUTOR.with_borrow(move |exec| {
            exec.clone().borrow().wake(*cloned);
        });
    }
}

pub fn get_next_task_id() -> Task {
    thread_local! {
        static NEXT_TASK_ID: Cell<u64> = const { Cell::new(1234u64) };
    }
    let out_id = NEXT_TASK_ID.get();
    NEXT_TASK_ID.set(out_id.wrapping_add(1));
    Task(out_id)
}

struct SqeFuture {
    shared: Rc<RefCell<SqeFutureShared>>,
}

//Same as an io_uring_cqe just without the user_data field
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StrippedCqe {
    pub res: i32,
    pub flags: u32,
}

impl From<&io_uring_cqe> for StrippedCqe {
    fn from(cqe: &io_uring_cqe) -> Self {
        Self {
            res: cqe.res,
            flags: cqe.flags,
        }
    }
}

struct SqeFutureShared {
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
    fn nop(exec: &Executor) -> SqeFuture {
        let shared = Rc::new(RefCell::new(SqeFutureShared {
            waker: None,
            cqe: None,
            completed: false,
        }));
        let shared_copy = Rc::clone(&shared);

        let sqe = exec.get_sqe();
        unsafe {
            io_uring_prep_nop(sqe);
        }

        exec.register_task(Task(sqe.user_data), shared_copy);
        SqeFuture { shared }
    }
    #[must_use]
    fn socket(
        exec: &Executor,
        domain: i32,
        sock_type: i32,
        protocol: i32,
        flags: u32,
    ) -> SqeFuture {
        let shared = Rc::new(RefCell::new(SqeFutureShared {
            waker: None,
            cqe: None,
            completed: false,
        }));
        let shared_copy = Rc::clone(&shared);

        let sqe = exec.get_sqe();
        unsafe {
            io_uring_prep_socket(sqe, domain, sock_type, protocol, flags);
        }

        exec.register_task(Task(sqe.user_data), shared_copy);
        SqeFuture { shared }
    }
    #[must_use]
    fn connect(
        exec: &Executor,
        sockfd: i32,
        addr: *const sockaddr,
        addrlen: socklen_t,
    ) -> SqeFuture {
        let shared = Rc::new(RefCell::new(SqeFutureShared {
            waker: None,
            cqe: None,
            completed: false,
        }));
        let shared_copy = Rc::clone(&shared);

        let sqe = exec.get_sqe();
        unsafe {
            io_uring_prep_connect(sqe, sockfd, addr, addrlen);
        }

        exec.register_task(Task(sqe.user_data), shared_copy);
        SqeFuture { shared }
    }
    #[must_use]
    fn send(exec: &Executor, sockfd: i32, buf: &[u8], flags: i32) -> SqeFuture {
        let shared = Rc::new(RefCell::new(SqeFutureShared {
            waker: None,
            cqe: None,
            completed: false,
        }));
        let shared_copy = Rc::clone(&shared);

        let sqe = exec.get_sqe();
        unsafe {
            io_uring_prep_send(sqe, sockfd, buf.as_ptr() as *const c_void, buf.len(), flags);
        }

        exec.register_task(Task(sqe.user_data), shared_copy);
        SqeFuture { shared }
    }
}

struct Ring {
    pub inner: RefCell<io_uring>,
    pub cq_buf: Vec<*mut io_uring_cqe>,
}

impl Ring {
    pub fn new(entries: u32, flags: u32) -> Result<Self, Error> {
        let cq_buf = Vec::with_capacity(entries as usize);
        unsafe {
            let mut ring: io_uring = std::mem::zeroed();
            match io_uring_queue_init(entries, &mut ring, flags) {
                0 => Ok(Ring {
                    inner: RefCell::new(ring),
                    cq_buf,
                }),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn get_sqe(&self) -> &mut io_uring_sqe {
        unsafe {
            match io_uring_get_sqe(self.inner.as_ptr()).as_mut() {
                Some(sqe) => {
                    let task_id = get_next_task_id().0;
                    println!("Creating SQE with ID: {task_id}");
                    io_uring_sqe_set_data64(sqe, task_id);
                    sqe
                }
                None => {
                    self.submit()
                        .expect("io_uring_submit() failed after NULL SQE");
                    self.get_sqe()
                }
            }
        }
    }
    pub fn submit(&self) -> Result<i32, Error> {
        /*
         * If SQPOLL is used, the return value may report a higher number of submitted entries
         * than actually submitted. If the user requires accurate information about how many
         * submission queue entries have been successfully submitted, while using SQPOLL,
         * the user must fall back to repeatedly submitting a single submission queue entry.
         */
        unsafe {
            match io_uring_submit(self.inner.as_ptr()) {
                n if n >= 0 => Ok(n),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
    pub fn submit_and_wait_timeout(&self, timeout: Option<Duration>) -> Result<i32, Error> {
        let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

        let mut rcts = Rc::new(__kernel_timespec {
            tv_sec: 0,
            tv_nsec: 0,
        });

        let ts = match timeout {
            Some(d) => {
                let mts = Rc::get_mut(&mut rcts).unwrap();
                *mts = __kernel_timespec {
                    tv_sec: d.as_secs() as i64,
                    tv_nsec: d.subsec_nanos() as i64,
                };
                mts
            }
            None => std::ptr::null_mut(),
        };

        unsafe {
            match io_uring_submit_and_wait_timeout(
                self.inner.as_ptr(),
                &mut cqe,
                1,
                ts,
                std::ptr::null_mut(),
            ) {
                n if n >= 0 => Ok(n),
                err => Err(Error::from_raw_os_error(-err)),
            }
        }
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe { io_uring_queue_exit(self.inner.as_ptr()) }
    }
}

struct Executor {
    ring: Ring,

    task_map: RefCell<HashMap<u64, Rc<RefCell<SqeFutureShared>>>>,
    future_map: RefCell<HashMap<u64, LocalBoxFuture<'static, ()>>>,

    task_queue: RefCell<VecDeque<u64>>,
}

impl Executor {
    pub fn new(entries: u32, flags: u32) -> Result<Self, Error> {
        let ring = Ring::new(entries, flags)?;
        let task_map = RefCell::new(HashMap::with_capacity(entries as usize));
        let future_map = RefCell::new(HashMap::with_capacity(entries as usize));
        let task_queue = RefCell::new(VecDeque::with_capacity(entries as usize));
        Ok(Executor {
            ring,
            task_map,
            future_map,
            task_queue,
        })
    }
    pub fn register_task(&self, task: Task, sqe: Rc<RefCell<SqeFutureShared>>) {
        self.task_map.borrow_mut().insert(task.0, sqe);
    }
    pub fn wake(&self, task: Task) {
        self.task_queue.borrow_mut().push_back(task.0);
    }
    pub fn get_sqe(&self) -> &mut io_uring_sqe {
        self.ring.get_sqe()
    }
    pub fn submit(&self) -> Result<i32, Error> {
        self.ring.submit()
    }
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        let future = future.boxed_local();

        let task = get_next_task_id();
        self.future_map.borrow_mut().insert(task.0, future);
        self.task_queue.borrow_mut().push_back(task.0);
    }
    pub fn run(&self) {
        loop {
            let mut map = self.future_map.borrow_mut();
            while let Some(task) = self.task_queue.borrow_mut().pop_front() {
                let future = map
                    .get_mut(&task)
                    .expect("Task queue contained an ID not in the future map");

                let waker = Waker::from(Arc::new(Task(task)));
                let context = &mut Context::from_waker(&waker);
                if future.as_mut().poll(context).is_ready() {
                    map.remove(&task);
                }
            }
            //No work in the queue to be done
            println!("No work in the queue!");

            loop {
                let mut cqe: *mut io_uring_cqe = std::ptr::null_mut();

                let res = self.ring.submit_and_wait_timeout(Some(Duration::new(1, 0)));
                match res {
                    Ok(_) => (),
                    Err(e) => {
                        let inner = e.raw_os_error().unwrap();
                        //62 is ETIME errno value
                        if inner == 62 {
                            println!("No more CQEs");
                            break;
                        }
                        panic!("Got an unknown error: {}", inner);
                    }
                };

                unsafe {
                    io_uring_peek_cqe(self.ring.inner.as_ptr(), &mut cqe);
                    //We peek after waiting, so there should always be a valid CQE to process here
                    assert!(!cqe.is_null());

                    //Got a CQE to process
                    println!("Got a CQE: {:#?}", *cqe);

                    let task_id = io_uring_cqe_get_data64(cqe);

                    println!("Waking task: {:#?}", task_id);

                    let task_map_binding = self.task_map.borrow();

                    let task = task_map_binding
                        .get(&task_id)
                        .expect("CQE user_data doesn't exist in the task map!");

                    task.borrow_mut().completed = true;
                    task.borrow_mut().cqe = Some(StrippedCqe::from(&*cqe));
                    task.borrow_mut()
                        .waker
                        .as_ref()
                        .expect("Got a completed task with no waker!")
                        .wake_by_ref();

                    io_uring_cqe_seen(self.ring.inner.as_ptr(), cqe);
                    println!("Done with task: {:#?}", task_id);
                }
            }
        }
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
    EXECUTOR.with_borrow(move |exec| {
        let inner = exec.clone();
        exec.clone().borrow().spawn(async move {
            println!("I am an async function!");

            let nop_result = SqeFuture::nop(&inner.borrow()).await;
            println!("CQE result: {nop_result:#?}");
            let socket_result = SqeFuture::socket(
                &inner.borrow(),
                AddressFamily::Inet as i32,
                SockType::Stream as i32,
                SockProtocol::Tcp as i32,
                0,
            )
            .await;
            println!("CQE result: {socket_result:#?}");

            let host = "127.0.0.1";
            let port = 8080;
            let addr = SockaddrIn::from_str(&format!("{}:{}", host, port)).unwrap();

            let connect_result = SqeFuture::connect(
                &inner.borrow(),
                socket_result.res,
                addr.as_ptr() as *const liburing_sys::sockaddr,
                addr.len(),
            )
            .await;
            println!("CQE result: {connect_result:#?}");

            let msg = "Hello io_uring world!\n";

            let send_result =
                SqeFuture::send(&inner.borrow(), socket_result.res, &msg.as_bytes(), 0).await;
            println!("CQE result: {connect_result:#?}");
        });
        exec.clone().borrow().run();
    });

    return;

    let cli = Cli::parse();

    //if cli.listen {
    //    server(cli.host, cli.port);
    //} else {
    //    client(cli.host, cli.port);
    //}
}
