// A channel which can only contain a single value at any given time, rather than a queue.
use std::{
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
    },
};

#[derive(Debug)]
pub enum ChannelErr {
    Lock,
    NoVal,
}

impl Error for ChannelErr {}

impl fmt::Display for ChannelErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ChannelErr::Lock => write!(f, "Lock poisoned"),
            ChannelErr::NoVal => write!(f, "No value found"),
        }
    }
}

type PErr<'a, T> = PoisonError<MutexGuard<'a, Option<T>>>;

impl<'a, T> From<PErr<'a, T>> for ChannelErr {
    fn from(_: PErr<'a, T>) -> ChannelErr {
        ChannelErr::Lock
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(Context::new());
    let sender = Sender(Arc::clone(&channel));
    let receiver = Receiver(Arc::clone(&channel));
    (sender, receiver)
}

#[derive(Debug)]
pub struct Context<T> {
    value: Mutex<Option<T>>,
    cvar: Condvar,
    closed: AtomicBool,
}

impl<T> Context<T> {
    pub fn new() -> Self {
        Self {
            value: Mutex::new(None),
            cvar: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }
}

impl<T> Default for Context<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct Sender<T>(Arc<Context<T>>);

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), ChannelErr> {
        let mut guard = self.0.value.lock()?;
        *guard = Some(value);
        self.0.cvar.notify_all();
        Ok(())
    }

    pub fn hangup(&self) {
        self.0.closed.swap(true, Ordering::Relaxed);
        self.0.cvar.notify_all();
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender(Arc::clone(&self.0))
    }
}

#[derive(Debug)]
pub struct Receiver<T>(Arc<Context<T>>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, ChannelErr> {
        let mut guard = self.0.value.lock()?;
        guard.take().ok_or(ChannelErr::NoVal)
    }

    pub fn recv(&self) -> Result<T, ChannelErr> {
        let mut guard = self.0.value.lock()?;
        while guard.is_none() {
            guard = self.0.cvar.wait(guard)?;
            if self.0.closed.load(Ordering::Relaxed) {
                return Err(ChannelErr::NoVal);
            }
        }
        Ok(guard.take().unwrap())
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(Arc::clone(&self.0))
    }
}
