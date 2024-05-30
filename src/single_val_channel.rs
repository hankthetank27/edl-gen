use std::{
    error::Error,
    fmt,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
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
    pub fn send(&self, value: T) -> Result<(), ChannelErr> {
        let mut guard = self.0.value.lock()?;
        *guard = Some(value);
        self.0.cvar.notify_all();
        Ok(())
    }
}

//TODO: use weak ptr and upgrade on send?
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender(Arc::clone(&self.0))
    }
}

pub struct Receiver<T>(Arc<Context<T>>);

impl<T: fmt::Debug> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, ChannelErr> {
        let mut guard = self.0.value.lock()?;
        guard.take().ok_or(ChannelErr::NoVal)
    }

    // TODO: see call in server
    pub fn recv(&self) -> Result<T, ChannelErr> {
        let mut guard = self.0.value.lock()?;
        while guard.is_none() {
            guard = self.0.cvar.wait(guard)?;
        }
        Ok(guard.take().unwrap())
    }
}

//TODO: use weak ptr and upgrade on recv?
impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(Arc::clone(&self.0))
    }
}
