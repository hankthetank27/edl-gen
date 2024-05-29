use std::sync::{Arc, Condvar, Mutex};

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(Context::new());
    let sender = Sender(Arc::clone(&channel));
    let receiver = Receiver(Arc::clone(&channel));
    (sender, receiver)
}

pub struct Context<T> {
    value: Mutex<Option<T>>,
    cvar: Condvar,
}

impl<T> Context<T> {
    pub fn new() -> Self {
        Self {
            value: Mutex::new(None),
            cvar: Condvar::new(),
        }
    }
}

impl<T> Default for Context<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Sender<T>(Arc<Context<T>>);

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        let mut guard = self.0.value.lock().unwrap();
        *guard = Some(value);
        self.0.cvar.notify_all();
    }
}

//TODO: use weak ptr and upgrade on send?
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender(Arc::clone(&self.0))
    }
}

pub struct Receiver<T>(Arc<Context<T>>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Option<T> {
        let mut guard = self.0.value.lock().unwrap();
        guard.take()
    }

    // TODO: see call in server
    pub fn recv(&self) -> T {
        let mut guard = self.0.value.lock().unwrap();
        while guard.is_none() {
            guard = self.0.cvar.wait(guard).unwrap();
        }
        guard.take().unwrap()
    }
}

//TODO: use weak ptr and upgrade on recv?
impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(Arc::clone(&self.0))
    }
}
