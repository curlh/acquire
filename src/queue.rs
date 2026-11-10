use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::Event;

type Source = Box<dyn Iterator<Item = Event> + Send>;

#[derive(Default)]
struct Inner {
    sources: VecDeque<Source>,
    closed: bool,
    canceled: bool,
}

/// Events waiting to be run. Events may be submitted while the engine runs, for example from a
/// hook that looks at the images, and iterators are read lazily, so a time series without end
/// is one `submit`.
#[derive(Clone, Default)]
pub struct Queue(Arc<(Mutex<Inner>, Condvar)>);

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit<I>(&self, events: I)
    where
        I: IntoIterator<Item = Event>,
        I::IntoIter: Send + 'static,
    {
        let (lock, cv) = &*self.0;
        lock.lock().unwrap().sources.push_back(Box::new(events.into_iter()));
        cv.notify_all();
    }

    /// No more events will come. The engine runs the ones waiting and returns.
    pub fn close(&self) {
        let (lock, cv) = &*self.0;
        lock.lock().unwrap().closed = true;
        cv.notify_all();
    }

    /// Stops the engine after the current step and drops the waiting events
    pub fn cancel(&self) {
        let (lock, cv) = &*self.0;
        lock.lock().unwrap().canceled = true;
        cv.notify_all();
    }

    pub(crate) fn is_canceled(&self) -> bool {
        self.0 .0.lock().unwrap().canceled
    }

    /// The next event. With `wait` it blocks until one is submitted or the queue is closed.
    pub(crate) fn next(&self, wait: bool) -> Option<Event> {
        let (lock, cv) = &*self.0;
        let mut inner = lock.lock().unwrap();
        loop {
            if inner.canceled {
                return None;
            }
            while let Some(source) = inner.sources.front_mut() {
                if let Some(event) = source.next() {
                    return Some(event);
                }
                inner.sources.pop_front();
            }
            if inner.closed || !wait {
                return None;
            }
            inner = cv.wait(inner).unwrap();
        }
    }
}
