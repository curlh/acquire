//! An adaptive acquisition: one overview image at each of 6 positions, and a Z stack only where
//! the overview is bright. The hook looks at each image and submits more events to the queue.

use acquire::*;

const POSITIONS: usize = 6;

struct ZStackWhereBright {
    queue: Queue,
    overviews_seen: usize,
}

impl Hook for ZStackWhereBright {
    fn after_camera(&mut self, _hw: &mut Hardware, o: &Output) -> Result<()> {
        // Overview images are the events without a Z index
        if o.event.z_index != 0 {
            return Ok(());
        }
        let Pixels::U16(pixels) = &o.frames[0].1.pixels else { return Err("expected 16 bit images".into()) };
        let mean = pixels.iter().map(|&p| p as f64).sum::<f64>() / pixels.len() as f64;
        let bright = mean > 3.0;
        println!("position {} mean {mean:.0}{}", o.event.position_index, if bright { ", taking a Z stack" } else { "" });
        if bright {
            let base = o.event.clone();
            self.queue.submit((1..=5).map(move |i| Event {
                z_index: i,
                z: Some(i as f64 - 3.0),
                ..base.clone()
            }));
        }
        self.overviews_seen += 1;
        if self.overviews_seen == POSITIONS {
            self.queue.close();
        }
        Ok(())
    }
}

/// Counts the images that reach the sink
struct Count(usize);

impl Sink for Count {
    fn write(&mut self, _o: Output) -> Result<()> {
        self.0 += 1;
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        println!("{} images", self.0);
        Ok(())
    }
}

fn main() -> Result<()> {
    let queue = Queue::new();
    queue.submit((0..POSITIONS).map(|p| Event {
        index: p as u64,
        position_index: p,
        xy: Some((p as f64 * 500.0, 0.0)),
        exposure_ms: 5.0,
        cameras: vec!["Cam".into()],
        open_shutter_before: true,
        close_shutter_after: true,
        ..Default::default()
    }));
    let hook = ZStackWhereBright { queue: queue.clone(), overviews_seen: 0 };
    let mut hw = sim::hardware();
    let outcome = run(&mut hw, &queue, &mut Count(0), &mut [Box::new(hook)])?;
    println!("{outcome:?}");
    Ok(())
}
