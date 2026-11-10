//! Acquisition core for microscopes. The lab writes its own devices,
//! UI and workflow; this crate does the timing, threading, hardware sequences and writing.
//!
//! Events are submitted to a `Queue`, `run` executes them on `Hardware`, `Hook`s run between the
//! hardware steps and the images go to a `Sink`. `Plan` expands a multi-dimensional acquisition
//! into events. `Microscope` owns the hardware on its own thread for programs with a UI and adds
//! live view. `OmeTiffSink` and `BinarySink` write the images. See AGENTS.md.

mod device;
mod engine;
mod error;
mod frame;
mod microscope;
mod ometiff;
mod plan;
mod queue;
pub mod sim;
mod sink;

pub use device::{Camera, Hardware, Presets, Properties, Shutter, Stage, XyStage};
pub use engine::{run, Hook, Outcome, Output, Sink};
pub use error::{Error, Result};
pub use frame::{Frame, Pixels};
pub use microscope::Microscope;
pub use ometiff::OmeTiffSink;
pub use plan::{Axis, Channel, Event, Plan, Position, Property};
pub use queue::Queue;
pub use sink::{BinarySink, Buffered};
