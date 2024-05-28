use std::sync::{
    atomic::{AtomicPtr, Ordering},
    Arc,
};

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(SingleValChannel::new());
    let sender = Sender(Arc::clone(&channel));
    let receiver = Receiver(Arc::clone(&channel));
    (sender, receiver)
}

pub struct SingleValChannel<T> {
    value: AtomicPtr<Option<T>>,
}

impl<T> SingleValChannel<T> {
    pub fn new() -> Self {
        Self {
            value: AtomicPtr::new(Box::into_raw(Box::new(None))),
        }
    }
}

impl<T> Default for SingleValChannel<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Sender<T>(Arc<SingleValChannel<T>>);

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        let ptr = Box::into_raw(Box::new(Some(value)));
        let old_ptr = self.0.value.swap(ptr, Ordering::SeqCst);

        unsafe {
            drop(Box::from_raw(old_ptr));
        }
    }
}
pub struct Receiver<T>(Arc<SingleValChannel<T>>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Option<T> {
        let ptr = self
            .0
            .value
            .swap(Box::into_raw(Box::new(None)), Ordering::AcqRel);

        unsafe { *Box::from_raw(ptr) }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(Arc::clone(&self.0))
    }
}
