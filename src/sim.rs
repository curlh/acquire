//! Simulated devices, for running and testing a program without hardware

use std::collections::HashMap;

use crate::*;

/// Gives 16 bit images whose pixels all hold the number of the image, and sleeps for the
/// exposure time
pub struct SimCamera {
    pub id: String,
    pub width: u32,
    pub height: u32,
    exposure_ms: f64,
    count: u16,
}

impl SimCamera {
    pub fn new(id: &str) -> Self {
        SimCamera { id: id.to_string(), width: 64, height: 48, exposure_ms: 10.0, count: 0 }
    }
}

impl Camera for SimCamera {
    fn id(&self) -> &str {
        &self.id
    }
    fn set_exposure(&mut self, ms: f64) -> Result<()> {
        self.exposure_ms = ms;
        Ok(())
    }
    fn snap(&mut self) -> Result<Frame> {
        std::thread::sleep(std::time::Duration::from_secs_f64(self.exposure_ms / 1000.0));
        self.count = self.count.wrapping_add(1);
        Ok(Frame {
            width: self.width,
            height: self.height,
            bits_per_sample: 16,
            pixels: Pixels::U16(vec![self.count; (self.width * self.height) as usize]),
            timestamp_ns: 0,
        })
    }
    fn sequenceable(&self) -> bool {
        true
    }
    fn start_sequence(&mut self, _count: usize) -> Result<()> {
        Ok(())
    }
    fn next_frame(&mut self) -> Result<Frame> {
        self.snap()
    }
}

#[derive(Default)]
pub struct SimXyStage(pub (f64, f64));

impl XyStage for SimXyStage {
    fn position(&mut self) -> Result<(f64, f64)> {
        Ok(self.0)
    }
    fn set_position(&mut self, x: f64, y: f64) -> Result<()> {
        self.0 = (x, y);
        Ok(())
    }
}

pub struct SimStage {
    pub id: String,
    pub position: f64,
}

impl SimStage {
    pub fn new(id: &str) -> Self {
        SimStage { id: id.to_string(), position: 0.0 }
    }
}

impl Stage for SimStage {
    fn id(&self) -> &str {
        &self.id
    }
    fn position(&mut self) -> Result<f64> {
        Ok(self.position)
    }
    fn set_position(&mut self, position: f64) -> Result<()> {
        self.position = position;
        Ok(())
    }
    fn sequence_max_len(&self) -> usize {
        64
    }
    fn load_sequence(&mut self, _positions: &[f64]) -> Result<()> {
        Ok(())
    }
    fn start_sequence(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct SimShutter(pub bool);

impl Shutter for SimShutter {
    fn set_open(&mut self, open: bool) -> Result<()> {
        self.0 = open;
        Ok(())
    }
}

#[derive(Default)]
pub struct SimPresets(pub HashMap<String, String>);

impl Presets for SimPresets {
    fn current(&mut self, group: &str) -> Result<String> {
        Ok(self.0.get(group).cloned().unwrap_or_default())
    }
    fn apply(&mut self, group: &str, preset: &str) -> Result<()> {
        self.0.insert(group.to_string(), preset.to_string());
        Ok(())
    }
}

#[derive(Default)]
pub struct SimProperties(pub HashMap<(String, String), String>);

impl Properties for SimProperties {
    fn get(&mut self, device: &str, property: &str) -> Result<String> {
        Ok(self.0.get(&(device.to_string(), property.to_string())).cloned().unwrap_or_default())
    }
    fn set(&mut self, device: &str, property: &str, value: &str) -> Result<()> {
        self.0.insert((device.to_string(), property.to_string()), value.to_string());
        Ok(())
    }
}

/// A simulated microscope: camera "Cam", XY stage, focus "Z", shutter, presets and properties
pub fn hardware() -> Hardware {
    Hardware {
        cameras: vec![Box::new(SimCamera::new("Cam"))],
        xy: Some(Box::new(SimXyStage::default())),
        z: Some(Box::new(SimStage::new("Z"))),
        stages: Vec::new(),
        shutter: Some(Box::new(SimShutter::default())),
        presets: Some(Box::new(SimPresets::default())),
        properties: Some(Box::new(SimProperties::default())),
    }
}
