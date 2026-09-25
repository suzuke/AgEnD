//! A minimal `block_on` that drives the boundary traits' futures on the
//! calling thread, so contract suites and fake-based tests need no async
//! runtime.
//!
//! A real implementation whose futures need a runtime (for example tokio I/O)
//! enters that runtime in its contract fixture; the futures are still polled
//! here.
//!
//! Must NOT: spawn threads or depend on an async runtime crate.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Polls `future` to completion on the current thread, parking between polls.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    #[test]
    fn ready_future_returns_its_value() {
        assert_eq!(block_on(async { 41 + 1 }), 42);
    }

    #[test]
    fn pending_future_is_polled_again_after_wake() {
        struct WakeLater {
            waker: Arc<Mutex<Option<Waker>>>,
            polled: bool,
        }
        impl Future for WakeLater {
            type Output = &'static str;
            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Self::Output> {
                if self.polled {
                    return Poll::Ready("woken");
                }
                self.polled = true;
                *self.waker.lock().unwrap() = Some(cx.waker().clone());
                Poll::Pending
            }
        }
        let slot = Arc::new(Mutex::new(None::<Waker>));
        let remote = Arc::clone(&slot);
        let waker_thread = thread::spawn(move || {
            loop {
                if let Some(waker) = remote.lock().unwrap().take() {
                    thread::sleep(Duration::from_millis(10));
                    waker.wake();
                    return;
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        let result = block_on(WakeLater {
            waker: slot,
            polled: false,
        });
        waker_thread.join().unwrap();
        assert_eq!(result, "woken");
    }
}
