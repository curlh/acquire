# acquire

Acquisition core for microscopes, written in Rust.

The lab writes its device drivers, its UI and its workflow, often with an AI agent. acquire is the part that is hard to get right and the same everywhere: timing of events, hardware sequences, threads, live view, putting the hardware back after a run, and writing images.

- Events go into a `Queue`, which takes more events while running and reads iterators lazily.
- Consecutive events that differ only in Z and exposure are taken as one hardware sequence when the devices support it.
- `Hook`s run between hardware steps and after each image, for autofocus and adaptive acquisition.
- `Microscope` owns the hardware on its own thread, for programs with a UI, and adds live view.
- `OmeTiffSink` writes OME-TIFF (BigTIFF, OME-XML 2016-06) and `BinarySink` raw frames with a CSV index, both in the layouts of ScopeWriter.
- `sim` has simulated devices, so a program runs without hardware.

AGENTS.md describes how to build a program on it.

    cargo run --example plan
    cargo run --example adaptive
    cargo run --example live

Licensed under either of MIT or Apache-2.0, at your option.
