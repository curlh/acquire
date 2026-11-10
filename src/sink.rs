use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::JoinHandle;

use crate::{Error, Output, Result, Sink};

// Same columns as the Binary format of ScopeWriter, so its readers open the files
const FRAME_HEADER: &str = "camera_id,frame_index,timestamp_ns,width,height,bits_per_sample,stride,pixel_format,pixel_format_id,payload_bytes,source_roi_x,source_roi_y,source_roi_width,source_roi_height";
const EVENT_HEADER: &str = "event,time,position,position_label,channel,z_index,x,y,z,exposure_ms,started_ns,completed_ns";

struct CameraFiles {
    raw: BufWriter<File>,
    csv: BufWriter<File>,
    frames: u64,
}

/// Writes raw frames with a CSV index in the Binary layout of ScopeWriter: `<camera>.raw` and
/// `<camera>.csv` for each camera, and `events.csv` with the axes and times of every event.
pub struct BinarySink {
    dir: PathBuf,
    cameras: HashMap<String, CameraFiles>,
    events: BufWriter<File>,
}

impl BinarySink {
    pub fn create(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(io)?;
        let mut events = BufWriter::new(File::create(dir.join("events.csv")).map_err(io)?);
        writeln!(events, "{EVENT_HEADER}").map_err(io)?;
        Ok(BinarySink { dir, cameras: HashMap::new(), events })
    }

    fn files(&mut self, camera: &str) -> Result<&mut CameraFiles> {
        if !self.cameras.contains_key(camera) {
            let raw = BufWriter::new(File::create(self.dir.join(format!("{camera}.raw"))).map_err(io)?);
            let mut csv = BufWriter::new(File::create(self.dir.join(format!("{camera}.csv"))).map_err(io)?);
            writeln!(csv, "{FRAME_HEADER}").map_err(io)?;
            self.cameras.insert(camera.to_string(), CameraFiles { raw, csv, frames: 0 });
        }
        Ok(self.cameras.get_mut(camera).unwrap())
    }
}

impl Sink for BinarySink {
    fn write(&mut self, o: Output) -> Result<()> {
        let e = &o.event;
        let opt = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
        let (x, y) = e.xy.unzip();
        writeln!(
            self.events,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            e.index, e.time_index, e.position_index, e.position_label.as_deref().unwrap_or(""), e.channel_index, e.z_index,
            opt(x), opt(y), opt(e.z), e.exposure_ms, o.started_ns, o.completed_ns
        )
        .map_err(io)?;
        for (camera, frame) in &o.frames {
            let files = self.files(camera)?;
            let bytes = frame.pixels.len() * frame.pixels.bytes();
            let (format, format_id) = frame.pixels.format();
            frame.pixels.write_le(&mut files.raw).map_err(io)?;
            writeln!(
                files.csv,
                "{},{},{},{},{},{},{},{},{},{},0,0,{},{}",
                camera, files.frames, frame.timestamp_ns, frame.width, frame.height, frame.bits_per_sample,
                frame.width as usize * frame.pixels.bytes(), format, format_id, bytes, frame.width, frame.height
            )
            .map_err(io)?;
            files.frames += 1;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.events.flush().map_err(io)?;
        for files in self.cameras.values_mut() {
            files.raw.flush().map_err(io)?;
            files.csv.flush().map_err(io)?;
        }
        Ok(())
    }
}

/// Runs a sink on its own thread behind a bounded queue, so that writing to disk does not
/// delay the next event. `write` blocks only when `capacity` outputs are waiting.
pub struct Buffered {
    tx: Option<SyncSender<Output>>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Buffered {
    pub fn new<S: Sink + Send + 'static>(mut sink: S, capacity: usize) -> Self {
        let (tx, rx) = sync_channel::<Output>(capacity);
        let worker = std::thread::spawn(move || {
            for output in rx {
                sink.write(output)?;
            }
            sink.finish()
        });
        Buffered { tx: Some(tx), worker: Some(worker) }
    }
}

impl Sink for Buffered {
    fn write(&mut self, output: Output) -> Result<()> {
        // A closed queue means the writer thread stopped on an error, which finish reports
        match self.tx.as_ref().unwrap().send(output) {
            Ok(()) => Ok(()),
            Err(_) => self.finish(),
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.tx = None;
        match self.worker.take() {
            Some(worker) => worker.join().map_err(|_| Error("writer thread panicked".into()))?,
            None => Ok(()),
        }
    }
}

fn io(e: std::io::Error) -> Error {
    Error(e.to_string())
}
