use std::io::{self, Write};

#[derive(Clone, Debug)]
pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    F32(Vec<f32>),
}

impl Pixels {
    pub fn len(&self) -> usize {
        match self {
            Pixels::U8(v) => v.len(),
            Pixels::U16(v) => v.len(),
            Pixels::U32(v) => v.len(),
            Pixels::F32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes per pixel
    pub fn bytes(&self) -> usize {
        match self {
            Pixels::U8(_) => 1,
            Pixels::U16(_) => 2,
            Pixels::U32(_) | Pixels::F32(_) => 4,
        }
    }

    /// Name and id of the pixel format in the Binary index of ScopeWriter
    pub(crate) fn format(&self) -> (&'static str, u32) {
        match self {
            Pixels::U8(_) => ("Mono8", 0),
            Pixels::U16(_) => ("Mono16", 1),
            Pixels::U32(_) => ("UInt32", 2),
            Pixels::F32(_) => ("Float32", 6),
        }
    }

    /// Writes the pixels as little endian bytes
    pub(crate) fn write_le(&self, w: &mut impl Write) -> io::Result<()> {
        let bytes: Vec<u8> = match self {
            Pixels::U8(v) => return w.write_all(v),
            Pixels::U16(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Pixels::U32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Pixels::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        };
        w.write_all(&bytes)
    }
}

/// One monochrome image from a camera
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Bits the camera uses of each pixel, for example 12 in a 16 bit pixel
    pub bits_per_sample: u8,
    pub pixels: Pixels,
    /// Time since the start of the run in nanoseconds, set by the engine
    pub timestamp_ns: u64,
}
