/// Axes of an acquisition. The order in a plan runs from the slowest to the fastest axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Time,
    Position,
    Channel,
    Z,
}

#[derive(Clone, Debug, Default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    /// Focus position. The Z positions of the plan are offsets from it.
    pub z: Option<f64>,
    pub label: Option<String>,
}

/// One preset of the channel group
#[derive(Clone, Debug)]
pub struct Channel {
    pub preset: String,
    /// None keeps the exposure of the plan
    pub exposure_ms: Option<f64>,
    /// Focus offset of the channel in micrometers
    pub z_offset_um: f64,
    /// False images this channel only at the middle Z position
    pub z_stack: bool,
    /// Images this channel at every (skip_frames + 1)th time point
    pub skip_frames: u32,
    /// Camera that takes this channel. None uses every camera of the plan.
    pub camera: Option<String>,
}

impl Default for Channel {
    fn default() -> Self {
        Channel { preset: String::new(), exposure_ms: None, z_offset_um: 0.0, z_stack: true, skip_frames: 0, camera: None }
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub cameras: Vec<String>,
    pub order: Vec<Axis>,
    pub time_points: u32,
    pub interval_ms: f64,
    pub exposure_ms: f64,
    pub positions: Vec<Position>,
    pub z_positions: Vec<f64>,
    pub channel_group: String,
    pub channels: Vec<Channel>,
    /// Keep the shutter open between the Z positions or between the channels
    pub keep_shutter_open_slices: bool,
    pub keep_shutter_open_channels: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            cameras: Vec::new(),
            order: vec![Axis::Time, Axis::Position, Axis::Channel, Axis::Z],
            time_points: 1,
            interval_ms: 0.0,
            exposure_ms: 10.0,
            positions: Vec::new(),
            z_positions: Vec::new(),
            channel_group: String::new(),
            channels: Vec::new(),
            keep_shutter_open_slices: false,
            keep_shutter_open_channels: false,
        }
    }
}

/// A value for one property of one device
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub device: String,
    pub name: String,
    pub value: String,
}

impl Property {
    pub fn new(device: &str, name: &str, value: &str) -> Self {
        Property { device: device.into(), name: name.into(), value: value.into() }
    }
}

/// One step of the acquisition: set the hardware as described and take an image on each camera
#[derive(Clone, Debug, Default)]
pub struct Event {
    pub index: u64,
    pub time_index: u32,
    pub position_index: usize,
    pub channel_index: usize,
    pub z_index: usize,
    pub xy: Option<(f64, f64)>,
    pub z: Option<f64>,
    pub exposure_ms: f64,
    pub min_start_ms: f64,
    pub cameras: Vec<String>,
    pub channel: Option<(String, String)>,
    /// Device properties set before the image, in order
    pub properties: Vec<Property>,
    /// Positions of further single axis stages, by stage id
    pub stages: Vec<(String, f64)>,
    pub position_label: Option<String>,
    /// Focus offset of the channel. An event without Z of its own is taken at the focus
    /// position from the start of the run plus this offset.
    pub z_offset_um: f64,
    pub open_shutter_before: bool,
    pub close_shutter_after: bool,
}

impl Plan {
    pub fn events(&self) -> Vec<Event> {
        let count = |axis: Axis| match axis {
            Axis::Time => self.time_points as usize,
            Axis::Position => self.positions.len().max(1),
            Axis::Channel => self.channels.len().max(1),
            Axis::Z => self.z_positions.len().max(1),
        };
        let z_count = count(Axis::Z);
        // A channel that does not image the whole Z stack images its middle position
        let middle_z = (z_count - 1) / 2;
        let mut events = Vec::new();
        let mut index = [0usize; 4];
        let slot = |axis: Axis| axis as usize;
        let total: usize = self.order.iter().map(|&a| count(a)).product();
        for n in 0..total {
            let mut rest = n;
            for &axis in self.order.iter().rev() {
                index[slot(axis)] = rest % count(axis);
                rest /= count(axis);
            }
            let (t, p, c, z) = (
                index[slot(Axis::Time)],
                index[slot(Axis::Position)],
                index[slot(Axis::Channel)],
                index[slot(Axis::Z)],
            );
            let channel = self.channels.get(c);
            if let Some(ch) = channel {
                if (!ch.z_stack && z != middle_z) || t % (ch.skip_frames as usize + 1) != 0 {
                    continue;
                }
            }
            let position = self.positions.get(p);
            let focus = position.and_then(|q| q.z);
            let has_z = !self.z_positions.is_empty() || focus.is_some();
            let mut event = Event {
                index: events.len() as u64,
                time_index: t as u32,
                position_index: p,
                channel_index: c,
                z_index: z,
                xy: position.map(|q| (q.x, q.y)),
                position_label: position.and_then(|q| q.label.clone()),
                exposure_ms: self.exposure_ms,
                min_start_ms: t as f64 * self.interval_ms,
                cameras: self.cameras.clone(),
                ..Event::default()
            };
            if has_z {
                event.z = Some(focus.unwrap_or(0.0) + self.z_positions.get(z).copied().unwrap_or(0.0));
            }
            if let Some(ch) = channel {
                event.channel = Some((self.channel_group.clone(), ch.preset.clone()));
                if let Some(ms) = ch.exposure_ms {
                    event.exposure_ms = ms;
                }
                match event.z.as_mut() {
                    Some(zz) => *zz += ch.z_offset_um,
                    None => event.z_offset_um = ch.z_offset_um,
                }
                if let Some(camera) = &ch.camera {
                    event.cameras = vec![camera.clone()];
                }
            }
            events.push(event);
        }
        self.mark_shutter(&mut events);
        events
    }

    fn mark_shutter(&self, events: &mut [Event]) {
        let joined = |a: &Event, b: &Event| {
            a.time_index == b.time_index
                && a.position_index == b.position_index
                && ((self.keep_shutter_open_slices && a.channel_index == b.channel_index)
                    || (self.keep_shutter_open_channels && a.z_index == b.z_index))
        };
        for i in 0..events.len() {
            let opens = i == 0 || !joined(&events[i - 1], &events[i]);
            let closes = i + 1 == events.len() || !joined(&events[i], &events[i + 1]);
            events[i].open_shutter_before = opens;
            events[i].close_shutter_after = closes;
        }
    }
}
