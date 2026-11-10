//! How a program with a UI uses the core: live view, a device call from the UI, then an
//! acquisition in the background while the UI keeps showing the newest image

use std::time::Duration;

use acquire::*;

fn show(scope: &Microscope) {
    if let Some(frame) = scope.latest("Cam") {
        let Pixels::U16(p) = &frame.pixels else { return };
        println!("  newest image {}x{}, first pixel {}", frame.width, frame.height, p[0]);
    }
}

fn main() -> Result<()> {
    let scope = Microscope::new(sim::hardware());

    scope.live(&["Cam"])?;
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(50));
        show(&scope);
    }

    // A device call from the UI runs on the hardware thread; live view pauses around it
    scope.with(|hw| hw.cameras[0].set_exposure(20.0))?;
    println!("exposure set to 20 ms");
    std::thread::sleep(Duration::from_millis(100));
    show(&scope);

    let plan = Plan { cameras: vec!["Cam".into()], time_points: 20, exposure_ms: 10.0, ..Default::default() };
    let queue = Queue::new();
    queue.submit(plan.events());
    queue.close();
    let done = scope.acquire(&queue, Buffered::new(BinarySink::create(std::env::temp_dir().join("acquire_live"))?, 8), Vec::new());
    loop {
        match done.recv_timeout(Duration::from_millis(60)) {
            Ok(result) => {
                println!("acquisition {:?}", result?);
                break;
            }
            Err(_) => show(&scope),
        }
    }
    Ok(())
}
