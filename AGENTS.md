# Building a microscope program on acquire

acquire is the acquisition core. It does the timing of events, the hardware sequences, the threads, putting the hardware back after a run, live view and writing images. The lab's program supplies the device drivers, the UI and the workflow. Write those, and leave the core as it is.

## Devices

Implement the traits in `src/device.rs` for each device and put the devices in a `Hardware`. The engine calls each device from one thread, one call at a time, and a call returns when the device has done it, so a stage returns after it arrived. Return `Err` with a message that says what went wrong at the device; the run stops and puts the hardware back.

| Device | Trait | Required | Optional, for speed |
| --- | --- | --- | --- |
| Camera | `Camera` | `id`, `set_exposure`, `snap` | `sequenceable`, `start_sequence`, `next_frame`, `stop_sequence`, exposure sequences |
| XY stage | `XyStage` | `position`, `set_position` | |
| Focus and other single axis stages | `Stage` | `id`, `position`, `set_position` | stage sequences |
| Shutter | `Shutter` | `set_open` | |
| Channel presets | `Presets` | `current`, `apply` | |
| Anything else | `Properties` | `get`, `set` | |

A device with no trait of its own, such as a laser, an LED or a filter wheel, goes through `Properties` by device and property name. The focus drive is `Hardware::z`, further stages are in `Hardware::stages`.

A camera that can stream images should implement the sequence methods. Without them everything works, but each image is a separate `snap`, and live view is slower.

Most cameras come with a C SDK. Declare in Rust only the few SDK functions, structs and constants the driver uses, from the SDK's own header, and link the SDK library from a `build.rs`. Give every failed SDK call an error message that names the call and the SDK's error code. Keep drivers that follow a vendor's header in the lab's own crate.

`acquire::sim` has a simulated device for each trait and `sim::hardware()` builds a whole microscope from them. Write and run the program against it first, then swap in the real devices.

## Running events

An `Event` says how to set the hardware (channel, exposure, properties, XY, Z, further stages, shutter) and which cameras take an image. Fields left at their defaults are not touched. Events go into a `Queue`; `close()` it when no more will come.

`Plan` builds the events of the usual time, position, channel and Z acquisition. For anything else, build `Event`s directly.

Two ways to run:

- A script calls `run(&mut hw, &queue, &mut sink, &mut hooks)`, which returns when the queue is closed and empty. See `examples/plan.rs`.
- A program with a UI makes a `Microscope`, which owns the hardware on its own thread. Use `live`, `latest`, `acquire` and `with`, and never call a device from the UI thread. See `examples/live.rs`.

## Adapting the workflow

A `Hook` runs on the engine thread. `before_hardware` sees the events of the next step and may change or remove them or move the hardware, which is where an autofocus goes. `after_camera` sees each image; it may submit more events to the queue, which is how an acquisition reacts to what it sees. See `examples/adaptive.rs`.

## Images

Images go to a `Sink`. `OmeTiffSink` writes one `<camera>.ome.tif` per camera in the layout of ScopeWriter: BigTIFF, OME-XML of the 2016-06 schema, one Image per position with dimension order XYZCT, channel names, stage positions, exposure and time of every plane. Fiji, napari and Bio-Formats open it with all axes; set `pixel_size_um` for a physical scale. `BinarySink` writes `<camera>.raw` and `<camera>.csv` in the Binary layout of ScopeWriter, and `events.csv` with the axes and times of each event. Wrap a slow sink in `Buffered` so that it runs on its own thread. A sink of your own, for display or analysis, implements `write` and `finish`.

## Checking

    cargo build
    cargo run --example plan
    cargo run --example adaptive
    cargo run --example live

