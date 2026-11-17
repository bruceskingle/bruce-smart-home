use std::{
    cell::{Cell, RefCell},
    future::{Future, poll_fn},
    rc::Rc,
    task::{Poll, Waker},
};

/// A request for a fetch to stop early, shared between the UI which sets it and the fetch which awaits it.
/// Clones share the same state.
#[derive(Clone, Default)]
pub struct CancelToken(Rc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.set(true);
        if let Some(waker) = self.0.waker.take() {
            waker.wake();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.get()
    }

    /// Resolves when cancel is called, only one task may wait at a time.
    pub fn cancelled(&self) -> impl Future<Output = ()> + '_ {
        poll_fn(|cx| {
            if self.is_cancelled() {
                Poll::Ready(())
            } else {
                *self.0.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
    }
}
