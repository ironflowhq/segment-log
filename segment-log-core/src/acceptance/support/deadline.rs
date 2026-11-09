use std::{sync::mpsc, thread, time::Duration};

pub(crate) struct Deadline {
    cancel: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Deadline {
    pub(crate) fn new(timeout: Duration, failure: impl FnOnce() + Send + 'static) -> Self {
        let (cancel, commands) = mpsc::channel();
        let worker = thread::Builder::new()
            .spawn(move || {
                if commands.recv_timeout(timeout).is_err() {
                    let (saved, done) = mpsc::channel();
                    let _ = thread::Builder::new().spawn(move || {
                        failure();
                        let _ = saved.send(());
                    });
                    let _ = done.recv_timeout(Duration::from_secs(1));
                    std::process::abort();
                }
            })
            .unwrap_or_else(|_| std::process::abort());
        Self {
            cancel,
            worker: Some(worker),
        }
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        self.worker.take().unwrap().join().unwrap();
    }
}
