use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::{Error, Frame, Output, Pixels, Result, Sink};

// TIFF field types
const ASCII: u16 = 2;
const SHORT: u16 = 3;
const LONG: u16 = 4;
const LONG8: u16 = 16;

/// Metadata of one image plane, for the OME-XML
struct Plane {
    ifd: usize,
    position: usize,
    z: usize,
    c: usize,
    t: usize,
    timestamp_ns: u64,
    exposure_ms: f64,
    xy: Option<(f64, f64)>,
    z_um: Option<f64>,
}

struct OmeFile {
    out: BufWriter<File>,
    pos: u64,
    width: u32,
    height: u32,
    bits_per_sample: u8,
    /// OME pixel type, bits per sample and TIFF SampleFormat
    kind: (&'static str, u16, u16),
    frame_bytes: u64,
    /// File offset of the count field of the ImageDescription entry in the first IFD
    description_entry: u64,
    /// File offset of the next IFD field of the last IFD
    last_next: u64,
    planes: Vec<Plane>,
}

/// Writes one OME-TIFF per camera, `<camera>.ome.tif`, as ScopeWriter does: BigTIFF with one
/// uncompressed plane per IFD and OME-XML of the 2016-06 schema in the description of the first
/// IFD. Each position is one Image with dimension order XYZCT. Planes that no event took, such as
/// a channel imaged at the middle Z only, have no TiffData, and readers show them as blank.
pub struct OmeTiffSink {
    dir: PathBuf,
    /// Pixel size in the sample plane, written as PhysicalSizeX and PhysicalSizeY
    pub pixel_size_um: Option<f64>,
    files: HashMap<String, OmeFile>,
    channel_names: BTreeMap<usize, String>,
    position_names: BTreeMap<usize, String>,
}

impl OmeTiffSink {
    pub fn create(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(io)?;
        Ok(OmeTiffSink {
            dir,
            pixel_size_um: None,
            files: HashMap::new(),
            channel_names: BTreeMap::new(),
            position_names: BTreeMap::new(),
        })
    }

    fn xml(&self, f: &OmeFile) -> String {
        let size = |axis: fn(&Plane) -> usize| f.planes.iter().map(axis).max().map_or(1, |m| m + 1);
        let (size_z, size_c, size_t) = (size(|p| p.z), size(|p| p.c), size(|p| p.t));
        let positions: BTreeSet<usize> = f.planes.iter().map(|p| p.position).collect();
        let mut x = String::new();
        x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        x.push_str("<OME xmlns=\"http://www.openmicroscopy.org/Schemas/OME/2016-06\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://www.openmicroscopy.org/Schemas/OME/2016-06 http://www.openmicroscopy.org/Schemas/OME/2016-06/ome.xsd\"");
        let _ = writeln!(x, " Creator=\"acquire {}\">", env!("CARGO_PKG_VERSION"));
        for &p in &positions {
            let planes: Vec<&Plane> = f.planes.iter().filter(|q| q.position == p).collect();
            let name = self.position_names.get(&p).cloned().unwrap_or_else(|| format!("Position {}", p + 1));
            let _ = writeln!(x, "  <Image ID=\"Image:{p}\" Name=\"{}\">", esc(&name));
            let _ = write!(x, "    <StageLabel Name=\"{}\"", esc(&name));
            if let Some((sx, sy)) = planes[0].xy {
                let _ = write!(x, " X=\"{sx}\" XUnit=\"µm\" Y=\"{sy}\" YUnit=\"µm\"");
            }
            if let Some(sz) = planes[0].z_um {
                let _ = write!(x, " Z=\"{sz}\" ZUnit=\"µm\"");
            }
            x.push_str("/>\n");
            let _ = write!(
                x,
                "    <Pixels ID=\"Pixels:{p}\" DimensionOrder=\"XYZCT\" Type=\"{}\" SizeX=\"{}\" SizeY=\"{}\" SizeZ=\"{size_z}\" SizeC=\"{size_c}\" SizeT=\"{size_t}\" BigEndian=\"false\" Interleaved=\"false\"",
                f.kind.0, f.width, f.height
            );
            if let Some(s) = self.pixel_size_um {
                let _ = write!(x, " PhysicalSizeX=\"{s}\" PhysicalSizeXUnit=\"µm\" PhysicalSizeY=\"{s}\" PhysicalSizeYUnit=\"µm\"");
            }
            if f.kind.0 != "float" {
                let _ = write!(x, " SignificantBits=\"{}\"", f.bits_per_sample);
            }
            x.push_str(">\n");
            for c in 0..size_c {
                let name = self.channel_names.get(&c).cloned().unwrap_or_else(|| format!("Channel {}", c + 1));
                let _ = writeln!(x, "      <Channel ID=\"Channel:{p}:{c}\" Name=\"{}\" SamplesPerPixel=\"1\"/>", esc(&name));
            }
            for q in &planes {
                let _ = writeln!(
                    x,
                    "      <TiffData IFD=\"{}\" FirstZ=\"{}\" FirstC=\"{}\" FirstT=\"{}\" PlaneCount=\"1\"/>",
                    q.ifd, q.z, q.c, q.t
                );
            }
            for q in &planes {
                let _ = write!(
                    x,
                    "      <Plane TheZ=\"{}\" TheC=\"{}\" TheT=\"{}\" DeltaT=\"{}\" DeltaTUnit=\"ms\" ExposureTime=\"{}\" ExposureTimeUnit=\"ms\"",
                    q.z, q.c, q.t, q.timestamp_ns as f64 / 1e6, q.exposure_ms
                );
                if let Some((px, py)) = q.xy {
                    let _ = write!(x, " PositionX=\"{px}\" PositionXUnit=\"µm\" PositionY=\"{py}\" PositionYUnit=\"µm\"");
                }
                if let Some(pz) = q.z_um {
                    let _ = write!(x, " PositionZ=\"{pz}\" PositionZUnit=\"µm\"");
                }
                x.push_str("/>\n");
            }
            x.push_str("    </Pixels>\n  </Image>\n");
        }
        x.push_str("</OME>\n");
        x
    }
}

impl OmeFile {
    fn create(path: &Path, frame: &Frame) -> Result<Self> {
        let kind = kind(&frame.pixels);
        let frame_bytes = (frame.pixels.len() * frame.pixels.bytes()) as u64;
        let mut out = BufWriter::new(File::create(path).map_err(io)?);
        // BigTIFF header; the first IFD follows the first plane
        out.write_all(b"II").map_err(io)?;
        for v in [43u16, 8, 0] {
            out.write_all(&v.to_le_bytes()).map_err(io)?;
        }
        out.write_all(&(16 + frame_bytes).to_le_bytes()).map_err(io)?;
        Ok(OmeFile {
            out,
            pos: 16,
            width: frame.width,
            height: frame.height,
            bits_per_sample: frame.bits_per_sample,
            kind,
            frame_bytes,
            description_entry: 0,
            last_next: 0,
            planes: Vec::new(),
        })
    }

    /// Writes the plane and its IFD. The next IFD follows the next plane, which has the same size,
    /// so its offset is known now and the file is written front to back.
    fn add(&mut self, frame: &Frame) -> io::Result<()> {
        let data = self.pos;
        frame.pixels.write_le(&mut self.out)?;
        self.pos += self.frame_bytes;
        let first = self.planes.is_empty();
        let mut entries: Vec<(u16, u16, u64, u64)> = vec![
            (256, LONG, 1, self.width as u64),
            (257, LONG, 1, self.height as u64),
            (258, SHORT, 1, self.kind.1 as u64),
            (259, SHORT, 1, 1),
            (262, SHORT, 1, 1),
        ];
        if first {
            // Filled in by finish, once the OME-XML is known
            entries.push((270, ASCII, 0, 0));
        }
        entries.extend([
            (273, LONG8, 1, data),
            (277, SHORT, 1, 1),
            (278, LONG, 1, self.height as u64),
            (279, LONG8, 1, self.frame_bytes),
            (339, SHORT, 1, self.kind.2 as u64),
        ]);
        let ifd = self.pos;
        let ifd_bytes = 8 + entries.len() as u64 * 20 + 8;
        self.out.write_all(&(entries.len() as u64).to_le_bytes())?;
        for (i, (tag, kind, count, value)) in entries.iter().enumerate() {
            if *tag == 270 {
                self.description_entry = ifd + 8 + i as u64 * 20 + 4;
            }
            self.out.write_all(&tag.to_le_bytes())?;
            self.out.write_all(&kind.to_le_bytes())?;
            self.out.write_all(&count.to_le_bytes())?;
            self.out.write_all(&value.to_le_bytes())?;
        }
        self.last_next = ifd + ifd_bytes - 8;
        self.out.write_all(&(ifd + ifd_bytes + self.frame_bytes).to_le_bytes())?;
        self.pos += ifd_bytes;
        Ok(())
    }

    /// Appends the OME-XML, points the first IFD at it and ends the IFD chain
    fn finish(&mut self, xml: &str) -> io::Result<()> {
        let at = self.pos;
        self.out.write_all(xml.as_bytes())?;
        self.out.write_all(&[0])?;
        self.out.seek(SeekFrom::Start(self.description_entry))?;
        self.out.write_all(&(xml.len() as u64 + 1).to_le_bytes())?;
        self.out.write_all(&at.to_le_bytes())?;
        self.out.seek(SeekFrom::Start(self.last_next))?;
        self.out.write_all(&0u64.to_le_bytes())?;
        self.out.flush()
    }
}

impl Sink for OmeTiffSink {
    fn write(&mut self, o: Output) -> Result<()> {
        let e = &o.event;
        if let Some((_, preset)) = &e.channel {
            self.channel_names.entry(e.channel_index).or_insert_with(|| preset.clone());
        }
        if let Some(label) = &e.position_label {
            self.position_names.entry(e.position_index).or_insert_with(|| label.clone());
        }
        for (camera, frame) in &o.frames {
            if !self.files.contains_key(camera) {
                let file = OmeFile::create(&self.dir.join(format!("{camera}.ome.tif")), frame)?;
                self.files.insert(camera.clone(), file);
            }
            let f = self.files.get_mut(camera).unwrap();
            if (frame.width, frame.height, kind(&frame.pixels)) != (f.width, f.height, f.kind) {
                return Err(Error(format!("camera {camera} changed its image size or pixel type during the acquisition")));
            }
            f.add(frame).map_err(io)?;
            let ifd = f.planes.len();
            f.planes.push(Plane {
                ifd,
                position: e.position_index,
                z: e.z_index,
                c: e.channel_index,
                t: e.time_index as usize,
                timestamp_ns: frame.timestamp_ns,
                exposure_ms: e.exposure_ms,
                xy: e.xy,
                z_um: e.z,
            });
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        let mut files = std::mem::take(&mut self.files);
        for f in files.values_mut() {
            let xml = self.xml(f);
            f.finish(&xml).map_err(io)?;
        }
        Ok(())
    }
}

/// OME pixel type, bits per sample and TIFF SampleFormat
fn kind(pixels: &Pixels) -> (&'static str, u16, u16) {
    match pixels {
        Pixels::U8(_) => ("uint8", 8, 1),
        Pixels::U16(_) => ("uint16", 16, 1),
        Pixels::U32(_) => ("uint32", 32, 1),
        Pixels::F32(_) => ("float", 32, 3),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn io(e: io::Error) -> Error {
    Error(e.to_string())
}
