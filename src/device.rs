//! Device traits. The engine calls every device from one thread, one call at a time, and each
//! call returns when the device has done it, so a stage returns after it arrived. Methods with a
//! default are optional; the defaults say that the device cannot do it.

use crate::{Error, Frame, Result};

fn unsupported<T>(what: &str) -> Result<T> {
    Err(Error(format!("{what} is not supported by this device")))
}

pub trait Camera: Send {
    fn id(&self) -> &str;
    fn set_exposure(&mut self, ms: f64) -> Result<()>;
    /// Takes one image and returns it
    fn snap(&mut self) -> Result<Frame>;
    /// Whether the camera can take a run of images on its own clock or on hardware triggers.
    /// The engine then merges events into one sequence, and live uses it.
    fn sequenceable(&self) -> bool {
        false
    }
    /// Starts taking `count` images; `usize::MAX` runs until `stop_sequence`
    fn start_sequence(&mut self, _count: usize) -> Result<()> {
        unsupported("sequence acquisition")
    }
    /// Waits for the next image of the running sequence. Returns every image of the sequence
    /// once and in order, and an error when an image was lost.
    fn next_frame(&mut self) -> Result<Frame> {
        unsupported("sequence acquisition")
    }
    fn stop_sequence(&mut self) -> Result<()> {
        Ok(())
    }
    /// Number of exposures the camera can step through within a sequence, 0 without
    fn exposure_sequence_max_len(&self) -> usize {
        0
    }
    fn load_exposure_sequence(&mut self, _ms: &[f64]) -> Result<()> {
        unsupported("exposure sequences")
    }
}

pub trait XyStage: Send {
    fn position(&mut self) -> Result<(f64, f64)>;
    fn set_position(&mut self, x: f64, y: f64) -> Result<()>;
}

/// A single axis stage, such as the focus drive or a light sheet scanner
pub trait Stage: Send {
    fn id(&self) -> &str;
    fn position(&mut self) -> Result<f64>;
    fn set_position(&mut self, position: f64) -> Result<()>;
    /// Number of positions the stage can step through on camera triggers, 0 without
    fn sequence_max_len(&self) -> usize {
        0
    }
    fn load_sequence(&mut self, _positions: &[f64]) -> Result<()> {
        unsupported("stage sequences")
    }
    fn start_sequence(&mut self) -> Result<()> {
        unsupported("stage sequences")
    }
    fn stop_sequence(&mut self) -> Result<()> {
        Ok(())
    }
}

pub trait Shutter: Send {
    fn set_open(&mut self, open: bool) -> Result<()>;
}

/// Presets of configuration groups, as in Micro-Manager. A channel is one preset of a group.
pub trait Presets: Send {
    fn current(&mut self, group: &str) -> Result<String>;
    fn apply(&mut self, group: &str, preset: &str) -> Result<()>;
}

/// Named properties of any device, for settings that have no trait of their own
pub trait Properties: Send {
    fn get(&mut self, device: &str, property: &str) -> Result<String>;
    fn set(&mut self, device: &str, property: &str, value: &str) -> Result<()>;
}

/// The devices of one microscope. Leave out what the microscope does not have.
#[derive(Default)]
pub struct Hardware {
    pub cameras: Vec<Box<dyn Camera>>,
    pub xy: Option<Box<dyn XyStage>>,
    /// The focus drive, which `Event::z` moves
    pub z: Option<Box<dyn Stage>>,
    /// Further single axis stages, which `Event::stages` moves by id
    pub stages: Vec<Box<dyn Stage>>,
    pub shutter: Option<Box<dyn Shutter>>,
    pub presets: Option<Box<dyn Presets>>,
    pub properties: Option<Box<dyn Properties>>,
}

pub(crate) fn camera<'a>(hw: &'a mut Hardware, id: &str) -> Result<&'a mut Box<dyn Camera>> {
    hw.cameras.iter_mut().find(|c| c.id() == id).ok_or_else(|| Error(format!("no camera with id {id}")))
}

pub(crate) fn stage<'a>(hw: &'a mut Hardware, id: &str) -> Result<&'a mut Box<dyn Stage>> {
    hw.stages.iter_mut().find(|s| s.id() == id).ok_or_else(|| Error(format!("no stage with id {id}")))
}
