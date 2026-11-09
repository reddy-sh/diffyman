//! Connection time limits: how long a client may take to send a request's
//! headers, and how long a kept-alive connection may stay silent between
//! requests.
//!
//! hyper's own header timeout starts whenever it waits for a request, so on
//! a kept-alive connection it would also be the idle limit. Here the two are
//! separate:
//!
//! - A new connection must deliver a whole request head within
//!   `header_read` of being accepted.
//! - After a response, the connection may stay silent for `idle`; once the
//!   first byte of the next request arrives, its head must be complete
//!   within `header_read`.
//! - While a request is handled, from its head to the end of its response
//!   body, neither applies (the request time limit does).
//!
//! [`TimedListener`] wraps every accepted stream in a [`TimedIo`], which
//! fails a pending read with `TimedOut` once the limit of the current phase
//! has passed; hyper then closes the connection. The [`track`] middleware
//! tells the connection when a request starts and when its response body
//! ends, through the [`Connection`] that `serve` attaches to each request.
//!
//! On the way out, [`TimedIo`] also replaces hyper's own bodiless answer to
//! a request head it could not parse with a typed error ([`crate::rejected`]).

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker, ready};
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::serve::{IncomingStream, Listener};
use http_body::{Frame, SizeHint};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, Sleep};

/// The two connection time limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// Time to send a whole request head: from the accept on a new
    /// connection, from its first byte on a kept-alive one.
    pub header_read: Duration,
    /// Time a kept-alive connection may stay silent between requests.
    pub idle: Duration,
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Waiting for a request head since `since`; `head_from` is when its
    /// first byte arrived (the accept, on a new connection).
    Waiting {
        since: Instant,
        head_from: Option<Instant>,
    },
    /// This many requests are being handled.
    Busy(usize),
}

#[derive(Debug)]
struct Shared {
    phase: Phase,
    /// The task waiting to read, woken when the limit that applies changes.
    reader: Option<Waker>,
    /// The first bytes read while waiting for a request head, up to
    /// [`HEAD_START_BYTES`]: what a typed answer to a head hyper refuses
    /// may look at ([`crate::rejected`]). `None` once a request was routed,
    /// so bytes read ahead during a request are never taken for its start.
    head_start: Option<Vec<u8>>,
}

/// How much of a request head's start a connection keeps.
const HEAD_START_BYTES: usize = 256;

/// One connection's state, shared by its stream and its requests.
#[derive(Clone, Debug)]
pub struct Connection {
    shared: Arc<Mutex<Shared>>,
    timeouts: Timeouts,
}

impl Connection {
    fn new(timeouts: Timeouts) -> Connection {
        let now = Instant::now();
        Connection {
            shared: Arc::new(Mutex::new(Shared {
                phase: Phase::Waiting {
                    since: now,
                    head_from: Some(now),
                },
                reader: None,
                head_start: Some(Vec::new()),
            })),
            timeouts,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `bytes` arrived: on a connection waiting between requests, the next
    /// request's head has started. Keeps the start of a head.
    fn saw_bytes(&self, bytes: &[u8]) {
        let mut shared = self.lock();
        if let Phase::Waiting {
            head_from: head_from @ None,
            ..
        } = &mut shared.phase
        {
            *head_from = Some(Instant::now());
        }
        if matches!(shared.phase, Phase::Waiting { .. })
            && let Some(start) = &mut shared.head_start
        {
            let room = HEAD_START_BYTES.saturating_sub(start.len());
            start.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
    }

    /// The start of the request head being read, if it is known.
    fn head_start(&self) -> Vec<u8> {
        self.lock().head_start.clone().unwrap_or_default()
    }

    /// Remembers the reading task and returns the deadline of the current
    /// phase, if one applies.
    fn park_reader(&self, waker: &Waker) -> Option<Instant> {
        let mut shared = self.lock();
        match &shared.reader {
            Some(stored) if stored.will_wake(waker) => {}
            _ => shared.reader = Some(waker.clone()),
        }
        match shared.phase {
            Phase::Waiting {
                head_from: Some(start),
                ..
            } => Some(start + self.timeouts.header_read),
            Phase::Waiting {
                since,
                head_from: None,
            } => Some(since + self.timeouts.idle),
            Phase::Busy(_) => None,
        }
    }

    fn request_started(&self) {
        let mut shared = self.lock();
        shared.head_start = None;
        shared.phase = match shared.phase {
            Phase::Waiting { .. } => Phase::Busy(1),
            Phase::Busy(n) => Phase::Busy(n + 1),
        };
    }

    fn request_finished(&self) {
        let mut shared = self.lock();
        shared.phase = match shared.phase {
            Phase::Busy(n) if n > 1 => Phase::Busy(n - 1),
            _ => {
                shared.head_start = Some(Vec::new());
                Phase::Waiting {
                    since: Instant::now(),
                    head_from: None,
                }
            }
        };
        // The reader may be parked without a deadline; let it set the idle one.
        if let Some(reader) = &shared.reader {
            reader.wake_by_ref();
        }
    }
}

impl Connected<IncomingStream<'_, TimedListener>> for Connection {
    fn connect_info(stream: IncomingStream<'_, TimedListener>) -> Connection {
        stream.io().connection.clone()
    }
}

/// A TCP listener whose connections keep the time limits.
#[derive(Debug)]
pub struct TimedListener {
    inner: TcpListener,
    timeouts: Timeouts,
}

impl TimedListener {
    /// Wraps `inner`.
    pub fn new(inner: TcpListener, timeouts: Timeouts) -> TimedListener {
        TimedListener { inner, timeouts }
    }
}

impl Listener for TimedListener {
    type Io = TimedIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (TimedIo, SocketAddr) {
        let (stream, addr) = Listener::accept(&mut self.inner).await;
        let io = TimedIo {
            stream,
            connection: Connection::new(self.timeouts),
            sleep: Box::pin(tokio::time::sleep(self.timeouts.header_read)),
            armed: None,
            replacement: Vec::new(),
            replaced: 0,
        };
        (io, addr)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// An accepted stream that fails a pending read once its time limit passed,
/// and writes a typed answer instead of hyper's bodiless one to a request
/// it could not parse.
#[derive(Debug)]
pub struct TimedIo {
    stream: TcpStream,
    connection: Connection,
    sleep: Pin<Box<Sleep>>,
    armed: Option<Instant>,
    /// A typed answer written in place of hyper's ([`crate::rejected`]),
    /// and how much of it has been written.
    replacement: Vec<u8>,
    replaced: usize,
}

impl TimedIo {
    /// Writes what is left of the replacement answer.
    fn poll_replacement(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.replaced < self.replacement.len() {
            let rest = &self.replacement[self.replaced..];
            match Pin::new(&mut self.stream).poll_write(cx, rest) {
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) => self.replaced += n,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    /// When `buf` is hyper's answer to a request it could not parse, takes
    /// it as written and starts writing the typed answer instead.
    fn replace(&mut self, cx: &mut Context<'_>, buf: &[u8]) -> Option<Poll<io::Result<usize>>> {
        let answer = crate::rejected::typed(buf, || self.connection.head_start())?;
        self.replacement = answer;
        self.replaced = 0;
        Some(match self.poll_replacement(cx) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            // The rest is written on the next write, flush or shutdown.
            Poll::Ready(Ok(())) | Poll::Pending => Poll::Ready(Ok(buf.len())),
        })
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let before = buf.filled().len();
        match Pin::new(&mut this.stream).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if buf.filled().len() > before {
                    this.connection.saw_bytes(&buf.filled()[before..]);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => {
                let Some(deadline) = this.connection.park_reader(cx.waker()) else {
                    return Poll::Pending;
                };
                if this.armed != Some(deadline) {
                    this.sleep.as_mut().reset(deadline);
                    this.armed = Some(deadline);
                }
                match this.sleep.as_mut().poll(cx) {
                    Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the connection passed its time limit",
                    ))),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

impl AsyncWrite for TimedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_replacement(cx))?;
        if let Some(replaced) = this.replace(cx, buf) {
            return replaced;
        }
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_replacement(cx))?;
        // hyper's answer to a parse error is a head alone, in one buffer.
        let mut filled = bufs.iter().filter(|b| !b.is_empty());
        if let (Some(only), None) = (filled.next(), filled.next())
            && let Some(replaced) = this.replace(cx, only)
        {
            return replaced;
        }
        Pin::new(&mut this.stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        ready!(this.poll_replacement(cx))?;
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        ready!(this.poll_replacement(cx))?;
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

/// Ends a request's busy phase when dropped.
struct Finished(Connection);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.request_finished();
    }
}

/// A response body that ends its request's busy phase when it is done or
/// dropped.
struct TrackedBody {
    inner: Body,
    finished: Option<Finished>,
}

impl HttpBody for TrackedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(polled, Poll::Ready(None | Some(Err(_)))) {
            drop(self.finished.take());
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Marks the connection busy from the request's head to the end of its
/// response body. Requests that did not come through [`TimedListener`]
/// (tests that call the router directly) pass through unchanged.
pub async fn track(request: Request, next: Next) -> Response {
    let Some(ConnectInfo(connection)) = request
        .extensions()
        .get::<ConnectInfo<Connection>>()
        .cloned()
    else {
        return next.run(request).await;
    };
    connection.request_started();
    let finished = Finished(connection);
    let response = next.run(request).await;
    response.map(|inner| {
        Body::new(TrackedBody {
            inner,
            finished: Some(finished),
        })
    })
}
