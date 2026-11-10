use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::device::{camera, stage};
use crate::{Event, Frame, Hardware, Queue, Result};

/// What one event produced
pub struct Output {
    pub event: Event,
    /// One image per camera of the event, by camera id
    pub frames: Vec<(String, Arc<Frame>)>,
    pub started_ns: u64,
    pub completed_ns: u64,
}

/// Receives the outputs in event order, for example a writer or a display
pub trait Sink: Send {
    fn write(&mut self, output: Output) -> Result<()>;
    /// Called once after the last event, also when the run was canceled or failed
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Code that runs on the engine thread between hardware steps. This is where a workflow
/// adapts: autofocus, decisions on the images, submitting more events to the queue.
pub trait Hook: Send {
    /// Runs before the hardware is set for one step, which is one event or the events merged
    /// into one hardware sequence. It may change or remove events and move the hardware; the
    /// engine sends every setting again afterwards.
    fn before_hardware(&mut self, _hw: &mut Hardware, _events: &mut Vec<Event>) -> Result<()> {
        Ok(())
    }
    /// Runs when the images of an event have arrived, before they go to the sink
    fn after_camera(&mut self, _hw: &mut Hardware, _output: &Output) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Finished,
    Canceled,
}

/// What the hardware was last set to, so that unchanged settings are not sent again
#[derive(Default)]
struct State {
    preset: Option<(String, String)>,
    exposure: Option<(f64, Vec<String>)>,
    properties: HashMap<(String, String), String>,
    xy: Option<(f64, f64)>,
    z: Option<f64>,
    stages: HashMap<String, f64>,
    shutter_open: bool,
}

impl State {
    fn forget(&mut self) {
        *self = State { shutter_open: self.shutter_open, ..State::default() };
    }
}

/// Hardware state before the run, which is restored afterwards. Everything but the stages of
/// `Hardware` is read when the run first changes it.
#[derive(Default)]
struct Initial {
    xy: Option<(f64, f64)>,
    z: Option<f64>,
    preset: Option<(String, String)>,
    properties: HashMap<(String, String), String>,
    stages: HashMap<String, f64>,
}

struct Run<'a> {
    hw: &'a mut Hardware,
    sink: &'a mut dyn Sink,
    hooks: &'a mut [Box<dyn Hook>],
    start: Instant,
    state: State,
    initial: Initial,
}

/// Runs the events of the queue on the calling thread until the queue is closed and empty, or
/// canceled. Consecutive events that differ only in Z and exposure and need no wait are taken as
/// one hardware sequence when the devices support it. The hardware is put back afterwards.
pub fn run(hw: &mut Hardware, queue: &Queue, sink: &mut dyn Sink, hooks: &mut [Box<dyn Hook>]) -> Result<Outcome> {
    let initial = Initial {
        xy: hw.xy.as_mut().map(|s| s.position()).transpose()?,
        z: hw.z.as_mut().map(|s| s.position()).transpose()?,
        ..Initial::default()
    };
    let mut r = Run { hw, sink, hooks, start: Instant::now(), state: State::default(), initial };
    let result = r.steps(queue);
    let restored = r.restore();
    let finished = r.sink.finish();
    let outcome = result?;
    restored?;
    finished?;
    Ok(outcome)
}

impl Run<'_> {
    fn steps(&mut self, queue: &Queue) -> Result<Outcome> {
        let mut pending: Option<Event> = None;
        loop {
            let Some(first) = pending.take().or_else(|| queue.next(true)) else {
                return Ok(if queue.is_canceled() { Outcome::Canceled } else { Outcome::Finished });
            };
            let mut step = vec![first];
            if self.sequenceable(&step[0]) {
                // Only events already submitted are merged, so a live queue is never waited on
                while let Some(next) = queue.next(false) {
                    if self.joins(&step, &next) {
                        step.push(next);
                    } else {
                        pending = Some(next);
                        break;
                    }
                }
            }
            if !self.hooks.is_empty() {
                for hook in self.hooks.iter_mut() {
                    hook.before_hardware(self.hw, &mut step)?;
                }
                self.state.forget();
            }
            // A hook may have changed the events so that they no longer make one sequence
            let merged = step.len() > 1 && (1..step.len()).all(|i| self.joins(&step[..i], &step[i]));
            if merged {
                if !wait_until(self.start, step[0].min_start_ms, queue) {
                    return Ok(Outcome::Canceled);
                }
                self.sequence(&step)?;
            } else {
                for event in &step {
                    if !wait_until(self.start, event.min_start_ms, queue) {
                        return Ok(Outcome::Canceled);
                    }
                    self.single(event)?;
                }
            }
        }
    }

    fn single(&mut self, event: &Event) -> Result<()> {
        let started_ns = self.now();
        self.apply(event)?;
        self.open_shutter(event.open_shutter_before)?;
        let mut frames = Vec::new();
        for id in &event.cameras {
            let mut frame = camera(self.hw, id)?.snap()?;
            frame.timestamp_ns = self.now();
            frames.push((id.clone(), Arc::new(frame)));
        }
        if event.close_shutter_after {
            self.close_shutter()?;
        }
        let completed_ns = self.now();
        self.deliver(Output { event: event.clone(), frames, started_ns, completed_ns })
    }

    fn sequence(&mut self, step: &[Event]) -> Result<()> {
        self.apply(&step[0])?;
        let exposures: Vec<f64> = step.iter().map(|e| e.exposure_ms).collect();
        if exposures.windows(2).any(|w| w[0] != w[1]) {
            for id in &step[0].cameras {
                camera(self.hw, id)?.load_exposure_sequence(&exposures)?;
            }
            self.state.exposure = None;
        }
        let zs: Vec<f64> = step.iter().filter_map(|e| self.z_target(e)).collect();
        let z_steps = zs.windows(2).any(|w| w[0] != w[1]);
        if z_steps {
            let stage = self.hw.z.as_mut().unwrap();
            stage.load_sequence(&zs)?;
            stage.start_sequence()?;
        }
        self.open_shutter(step.iter().any(|e| e.open_shutter_before))?;
        let result = self.take_sequence(step);
        for id in &step[0].cameras {
            camera(self.hw, id)?.stop_sequence()?;
        }
        if z_steps {
            self.hw.z.as_mut().unwrap().stop_sequence()?;
            self.state.z = zs.last().copied();
        }
        if step.last().unwrap().close_shutter_after {
            self.close_shutter()?;
        }
        result
    }

    fn take_sequence(&mut self, step: &[Event]) -> Result<()> {
        for id in &step[0].cameras {
            camera(self.hw, id)?.start_sequence(step.len())?;
        }
        let mut started_ns = self.now();
        for event in step {
            let mut frames = Vec::new();
            for id in &event.cameras {
                let mut frame = camera(self.hw, id)?.next_frame()?;
                frame.timestamp_ns = self.now();
                frames.push((id.clone(), Arc::new(frame)));
            }
            let completed_ns = self.now();
            self.deliver(Output { event: event.clone(), frames, started_ns, completed_ns })?;
            started_ns = completed_ns;
        }
        Ok(())
    }

    fn deliver(&mut self, output: Output) -> Result<()> {
        for hook in self.hooks.iter_mut() {
            hook.after_camera(self.hw, &output)?;
        }
        self.sink.write(output)
    }

    /// Sends the settings of an event, skipping what is already set
    fn apply(&mut self, event: &Event) -> Result<()> {
        if let Some((group, preset)) = &event.channel {
            let p = self.hw.presets.as_mut().ok_or("the event has a channel but the hardware has no presets")?;
            if self.initial.preset.is_none() {
                self.initial.preset = Some((group.clone(), p.current(group)?));
            }
            if self.state.preset.as_ref() != Some(&(group.clone(), preset.clone())) {
                p.apply(group, preset)?;
                self.state.preset = Some((group.clone(), preset.clone()));
            }
        }
        for prop in &event.properties {
            let key = (prop.device.clone(), prop.name.clone());
            if self.state.properties.get(&key) != Some(&prop.value) {
                let p = self.hw.properties.as_mut().ok_or("the event sets properties but the hardware has no properties")?;
                if !self.initial.properties.contains_key(&key) {
                    self.initial.properties.insert(key.clone(), p.get(&prop.device, &prop.name)?);
                }
                p.set(&prop.device, &prop.name, &prop.value)?;
                self.state.properties.insert(key, prop.value.clone());
            }
        }
        if self.state.exposure.as_ref() != Some(&(event.exposure_ms, event.cameras.clone())) {
            for id in &event.cameras {
                camera(self.hw, id)?.set_exposure(event.exposure_ms)?;
            }
            self.state.exposure = Some((event.exposure_ms, event.cameras.clone()));
        }
        if let Some(xy) = event.xy {
            if self.state.xy != Some(xy) {
                let s = self.hw.xy.as_mut().ok_or("the event has an XY position but the hardware has no XY stage")?;
                s.set_position(xy.0, xy.1)?;
                self.state.xy = Some(xy);
            }
        }
        if let Some(z) = self.z_target(event) {
            if self.state.z != Some(z) {
                let s = self.hw.z.as_mut().ok_or("the event has a Z position but the hardware has no Z stage")?;
                s.set_position(z)?;
                self.state.z = Some(z);
            }
        }
        for (id, position) in &event.stages {
            if self.state.stages.get(id) != Some(position) {
                let s = stage(self.hw, id)?;
                if !self.initial.stages.contains_key(id) {
                    self.initial.stages.insert(id.clone(), s.position()?);
                }
                s.set_position(*position)?;
                self.state.stages.insert(id.clone(), *position);
            }
        }
        Ok(())
    }

    /// The focus position of an event. Without Z of its own it is the focus position from the
    /// start of the run plus the channel offset.
    fn z_target(&self, event: &Event) -> Option<f64> {
        match (event.z, self.initial.z) {
            (Some(z), _) => Some(z),
            (None, Some(z0)) => Some(z0 + event.z_offset_um),
            (None, None) => None,
        }
    }

    fn sequenceable(&self, event: &Event) -> bool {
        !event.cameras.is_empty()
            && event.cameras.iter().all(|id| self.hw.cameras.iter().any(|c| c.id() == id && c.sequenceable()))
    }

    /// Whether `next` can follow the events of `step` in one hardware sequence
    fn joins(&self, step: &[Event], next: &Event) -> bool {
        let first = &step[0];
        let same = next.cameras == first.cameras
            && next.channel == first.channel
            && next.properties == first.properties
            && next.xy == first.xy
            && next.stages == first.stages
            && next.min_start_ms <= first.min_start_ms;
        if !same {
            return false;
        }
        let n = step.len() + 1;
        let exposure_ok = step.iter().all(|e| e.exposure_ms == next.exposure_ms)
            || first.cameras.iter().all(|id| {
                self.hw.cameras.iter().any(|c| c.id() == id && c.exposure_sequence_max_len() >= n)
            });
        let z_next = self.z_target(next);
        let z_ok = step.iter().all(|e| self.z_target(e) == z_next)
            || (z_next.is_some() && self.hw.z.as_ref().map_or(0, |s| s.sequence_max_len()) >= n);
        exposure_ok && z_ok
    }

    fn open_shutter(&mut self, wanted: bool) -> Result<()> {
        if wanted && !self.state.shutter_open {
            if let Some(s) = self.hw.shutter.as_mut() {
                s.set_open(true)?;
                self.state.shutter_open = true;
            }
        }
        Ok(())
    }

    fn close_shutter(&mut self) -> Result<()> {
        if self.state.shutter_open {
            if let Some(s) = self.hw.shutter.as_mut() {
                s.set_open(false)?;
            }
            self.state.shutter_open = false;
        }
        Ok(())
    }

    /// Puts the hardware back, in the reverse order of `apply`
    fn restore(&mut self) -> Result<()> {
        self.close_shutter()?;
        for (id, position) in &self.initial.stages {
            stage(self.hw, id)?.set_position(*position)?;
        }
        if let (Some(z), Some(s)) = (self.initial.z, self.hw.z.as_mut()) {
            s.set_position(z)?;
        }
        if let (Some((x, y)), Some(s)) = (self.initial.xy, self.hw.xy.as_mut()) {
            s.set_position(x, y)?;
        }
        if let Some(p) = self.hw.properties.as_mut() {
            for ((device, name), value) in &self.initial.properties {
                p.set(device, name, value)?;
            }
        }
        if let (Some((group, preset)), Some(p)) = (&self.initial.preset, self.hw.presets.as_mut()) {
            p.apply(group, preset)?;
        }
        Ok(())
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }
}

/// Waits until an event may start. False when the queue was canceled meanwhile.
fn wait_until(start: Instant, ms: f64, queue: &Queue) -> bool {
    let target = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
    loop {
        if queue.is_canceled() {
            return false;
        }
        let elapsed = start.elapsed();
        if elapsed >= target {
            return true;
        }
        std::thread::sleep((target - elapsed).min(Duration::from_millis(20)));
    }
}
