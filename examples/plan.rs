//! A multi-dimensional acquisition from a script: 2 time points, 2 positions, a Z stack in DAPI
//! and one plane in FITC, written to OME-TIFF

use acquire::*;

/// Prints each hardware step and each image
struct Print;

impl Hook for Print {
    fn before_hardware(&mut self, _hw: &mut Hardware, events: &mut Vec<Event>) -> Result<()> {
        println!("step of {} event(s)", events.len());
        Ok(())
    }
    fn after_camera(&mut self, _hw: &mut Hardware, o: &Output) -> Result<()> {
        let e = &o.event;
        println!(
            "  #{:<3} t={} p={} c={:?} z={:?} {:>8.1} ms",
            e.index,
            e.time_index,
            e.position_index,
            e.channel.as_ref().map(|c| c.1.as_str()),
            e.z,
            o.frames[0].1.timestamp_ns as f64 / 1e6
        );
        Ok(())
    }
}

fn main() -> Result<()> {
    let plan = Plan {
        cameras: vec!["Cam".into()],
        time_points: 2,
        interval_ms: 300.0,
        exposure_ms: 5.0,
        positions: vec![
            Position { x: 0.0, y: 0.0, label: Some("A1".into()), ..Default::default() },
            Position { x: 100.0, y: 0.0, label: Some("A2".into()), ..Default::default() },
        ],
        z_positions: vec![-2.0, -1.0, 0.0, 1.0, 2.0],
        channel_group: "Channel".into(),
        channels: vec![
            Channel { preset: "DAPI".into(), ..Default::default() },
            Channel { preset: "FITC".into(), z_stack: false, exposure_ms: Some(8.0), ..Default::default() },
        ],
        keep_shutter_open_slices: true,
        ..Default::default()
    };
    let mut hw = sim::hardware();
    let queue = Queue::new();
    queue.submit(plan.events());
    queue.close();
    let mut tiff = OmeTiffSink::create(std::env::temp_dir().join("acquire_plan"))?;
    tiff.pixel_size_um = Some(0.108);
    let mut sink = Buffered::new(tiff, 8);
    let outcome = run(&mut hw, &queue, &mut sink, &mut [Box::new(Print)])?;
    println!("{outcome:?}");
    Ok(())
}
