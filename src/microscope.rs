use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::device::camera;
use crate::{run, Error, Frame, Hardware, Hook, Outcome, Output, Queue, Result, Sink};

type Latest = Arc<Mutex<HashMap<String, Arc<Frame>>>>;

enum Command {
    Live(Vec<String>, Sender<Result<()>>),
    StopLive(Sender<Result<()>>),
    Acquire { queue: Queue, sink: Box<dyn Sink>, hooks: Vec<Box<dyn Hook>>, done: Sender<Result<Outcome>> },
    With(Box<dyn FnOnce(&mut Hardware) + Send>),
}

/// Owns the hardware on a thread of its own, so that a UI never blocks on a device. Live view,
/// acquisitions and device calls from the UI all go through it, one at a time. The newest image
/// of every camera, from live or from an acquisition, is in `latest`.
pub struct Microscope {
    tx: Sender<Command>,
    latest: Latest,
    worker: Option<JoinHandle<()>>,
}

impl Microscope {
    pub fn new(hw: Hardware) -> Self {
        let (tx, rx) = channel();
        let latest = Latest::default();
        let mut worker = Worker { hw, live: Vec::new(), live_error: None, latest: latest.clone() };
        let worker = std::thread::spawn(move || worker.serve(rx));
        Microscope { tx, latest, worker: Some(worker) }
    }

    /// Starts live view on the given cameras, replacing the live view that runs
    pub fn live(&self, cameras: &[&str]) -> Result<()> {
        let (reply, rx) = channel();
        self.send(Command::Live(cameras.iter().map(|c| c.to_string()).collect(), reply));
        rx.recv().unwrap()
    }

    /// Stops live view and returns the error that stopped it early, if one did
    pub fn stop_live(&self) -> Result<()> {
        let (reply, rx) = channel();
        self.send(Command::StopLive(reply));
        rx.recv().unwrap()
    }

    /// The newest image of a camera
    pub fn latest(&self, camera: &str) -> Option<Arc<Frame>> {
        self.latest.lock().unwrap().get(camera).cloned()
    }

    /// Starts running the events of the queue and returns at once. Live view stops first. The
    /// result arrives on the returned receiver; `queue.cancel()` stops the acquisition.
    pub fn acquire(&self, queue: &Queue, sink: impl Sink + 'static, hooks: Vec<Box<dyn Hook>>) -> Receiver<Result<Outcome>> {
        let (done, rx) = channel();
        self.send(Command::Acquire { queue: queue.clone(), sink: Box::new(sink), hooks, done });
        rx
    }

    /// Runs `f` on the hardware thread and returns its result, for device calls from a UI. Live
    /// view pauses around it. It waits while an acquisition runs.
    pub fn with<R: Send + 'static>(&self, f: impl FnOnce(&mut Hardware) -> R + Send + 'static) -> R {
        let (reply, rx) = channel();
        self.send(Command::With(Box::new(move |hw| {
            let _ = reply.send(f(hw));
        })));
        rx.recv().unwrap()
    }

    fn send(&self, command: Command) {
        self.tx.send(command).expect("the hardware thread has stopped");
    }
}

impl Drop for Microscope {
    fn drop(&mut self) {
        // Closing the channel ends the hardware thread, which stops live view first
        self.tx = channel().0;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker {
    hw: Hardware,
    live: Vec<String>,
    live_error: Option<Error>,
    latest: Latest,
}

impl Worker {
    fn serve(&mut self, rx: Receiver<Command>) {
        loop {
            let command = if self.live.is_empty() {
                match rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => break,
                }
            } else {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => break,
                }
            };
            match command {
                Some(Command::Live(ids, reply)) => {
                    self.stop_live();
                    self.live_error = None;
                    let _ = reply.send(self.start_live(ids));
                }
                Some(Command::StopLive(reply)) => {
                    self.stop_live();
                    let _ = reply.send(self.live_error.take().map_or(Ok(()), Err));
                }
                Some(Command::Acquire { queue, sink, mut hooks, done }) => {
                    self.stop_live();
                    let mut sink = ShowLatest { inner: sink, latest: self.latest.clone() };
                    let _ = done.send(run(&mut self.hw, &queue, &mut sink, &mut hooks));
                }
                Some(Command::With(f)) => {
                    let ids = self.live.clone();
                    self.stop_live();
                    f(&mut self.hw);
                    if !ids.is_empty() {
                        if let Err(e) = self.start_live(ids) {
                            self.live_error = Some(e);
                        }
                    }
                }
                None => {
                    if let Err(e) = self.live_frame() {
                        self.live_error = Some(e);
                        self.stop_live();
                    }
                }
            }
        }
        self.stop_live();
    }

    fn start_live(&mut self, ids: Vec<String>) -> Result<()> {
        for id in &ids {
            let c = camera(&mut self.hw, id)?;
            if c.sequenceable() {
                c.start_sequence(usize::MAX)?;
            }
        }
        self.live = ids;
        Ok(())
    }

    fn stop_live(&mut self) {
        for id in std::mem::take(&mut self.live) {
            let stopped = camera(&mut self.hw, &id).and_then(|c| if c.sequenceable() { c.stop_sequence() } else { Ok(()) });
            if let Err(e) = stopped {
                self.live_error.get_or_insert(e);
            }
        }
    }

    fn live_frame(&mut self) -> Result<()> {
        for id in &self.live {
            let c = camera(&mut self.hw, id)?;
            let frame = if c.sequenceable() { c.next_frame()? } else { c.snap()? };
            self.latest.lock().unwrap().insert(id.clone(), Arc::new(frame));
        }
        Ok(())
    }
}

/// Puts the images of an acquisition into `latest` on their way to the sink
struct ShowLatest {
    inner: Box<dyn Sink>,
    latest: Latest,
}

impl Sink for ShowLatest {
    fn write(&mut self, output: Output) -> Result<()> {
        let mut latest = self.latest.lock().unwrap();
        for (id, frame) in &output.frames {
            latest.insert(id.clone(), frame.clone());
        }
        drop(latest);
        self.inner.write(output)
    }

    fn finish(&mut self) -> Result<()> {
        self.inner.finish()
    }
}
