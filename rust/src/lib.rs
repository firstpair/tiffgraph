//! Bounded TIFF/DNG graph walker, the Rust build of tiffgraph.
//!
//! It locates a DNG's primary XMP packet under a fixed grammar, protects every
//! other declared extent so the packet may not alias it, and reports the file's
//! physical regions. It is not a DNG validator, a decoder or a file identity.
//!
//! This is a line-for-line port of the OCaml `Tiffgraph` module: the same
//! grammars, the same limits, the same order of checks and the same refusal
//! reasons, so the two builds agree on every file (`test/cross.py`). The
//! grammars are the ones Semantic Unison negotiates as its
//! `org.firstpair.dng-xmp/0`, `/1` and `/2` profiles; changing what one admits
//! changes those profiles.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::RwLock;

/// Where the bytes come from. `read` must return exactly `length` bytes or
/// fail; `check` runs once, after a walk that succeeds (to confirm, say, that
/// the file did not change while it was read). Reader errors pass through
/// [`walk`] unchanged as [`WalkError::Reader`].
pub trait Reader {
    type Error;
    fn size(&self) -> i64;
    fn read(&mut self, offset: i64, length: usize) -> Result<Vec<u8>, Self::Error>;
    fn check(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A reader over bytes in memory.
pub struct SliceReader<'a>(pub &'a [u8]);

impl Reader for SliceReader<'_> {
    type Error = String;
    fn size(&self) -> i64 {
        self.0.len() as i64
    }
    fn read(&mut self, offset: i64, length: usize) -> Result<Vec<u8>, String> {
        let start = usize::try_from(offset).map_err(|_| "read before the start".to_string())?;
        let end = start.checked_add(length).filter(|&e| e <= self.0.len()).ok_or("read outside the bytes")?;
        Ok(self.0[start..end].to_vec())
    }
}

/// A reader over an open file, with positional reads.
#[cfg(unix)]
pub struct FileReader {
    file: std::fs::File,
    size: i64,
}

#[cfg(unix)]
impl FileReader {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len() as i64;
        Ok(FileReader { file, size })
    }
}

#[cfg(unix)]
impl Reader for FileReader {
    type Error = std::io::Error;
    fn size(&self) -> i64 {
        self.size
    }
    fn read(&mut self, offset: i64, length: usize) -> std::io::Result<Vec<u8>> {
        use std::os::unix::fs::FileExt;
        let mut buf = vec![0; length];
        self.file.read_exact_at(&mut buf, offset as u64)?;
        Ok(buf)
    }
}

/// The primary XMP packet's extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xmp {
    pub offset: i64,
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentRole {
    ImageBytes,
    StandardValue,
    PrivateValue,
    Structure,
}

/// One physical claim in the file, as the walk found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentRegion {
    pub logical_id: String,
    pub schema: String,
    pub role: ComponentRole,
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub extended: bool,
    pub opaque_maker: bool,
    pub layout_only: bool,
    pub require_xmp: bool,
    pub max_xmp: usize,
}

impl Policy {
    /// Classic TIFF, a fixed set of standard tags, one BYTE XMP packet.
    pub fn v0(max_xmp: usize) -> Policy {
        Policy { extended: false, opaque_maker: false, layout_only: false, require_xmp: true, max_xmp }
    }
    /// `v0` plus BigTIFF, embedded camera profiles and the DNG 1.7 JPEG XL scalars.
    pub fn v1(max_xmp: usize) -> Policy {
        Policy { extended: true, ..Policy::v0(max_xmp) }
    }
    /// `v1` plus the private fields camera converters write, kept as protected values.
    pub fn merge_only(max_xmp: usize) -> Policy {
        Policy { opaque_maker: true, ..Policy::v1(max_xmp) }
    }
    /// `v1` geometry with any field kept byte-exact; no merge authority.
    pub fn layout(max_xmp: usize) -> Policy {
        Policy { layout_only: true, ..Policy::v1(max_xmp) }
    }
}

/// Why a walk stopped: the file is outside the grammar, or the reader failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkError<E> {
    Refused(String),
    Reader(E),
}

impl<E: fmt::Display> fmt::Display for WalkError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalkError::Refused(s) => write!(f, "{s}"),
            WalkError::Reader(e) => write!(f, "{e}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for WalkError<E> {}

type Step<T, E> = Result<T, WalkError<E>>;

fn reject<T, E>(s: &str) -> Step<T, E> {
    Err(WalkError::Refused(s.to_string()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Image,
    Exif,
    Gps,
    Interop,
    CameraProfile,
}

#[derive(Clone, Copy)]
struct Layout {
    big: bool,
    count_width: usize,
    entry_size: usize,
    number_width: usize,
    inline_width: usize,
    value_at: usize,
}

const CLASSIC: Layout = Layout { big: false, count_width: 2, entry_size: 12, number_width: 4, inline_width: 4, value_at: 8 };
const BIGTIFF: Layout = Layout { big: true, count_width: 8, entry_size: 20, number_width: 8, inline_width: 8, value_at: 12 };

#[derive(Clone, Copy)]
struct Directory {
    kind: Kind,
    little: bool,
    layout: Layout,
    base: i64,
    offset: i64,
}

enum Pending {
    Directory(Directory),
    CameraHeader(i64),
}

// Values in this deliberately finite profile either live wholly in their TIFF
// value extent or use one of the graph/image-offset rules below. Unknown fields
// are not safe to treat as ordinary values: private LONGs and opaque MakerNotes
// can contain offsets into the proposed XMP packet.
fn image_value_tag(tag: i64) -> bool {
    matches!(tag,
        254 | 255 | 256 | 257 | 258 | 259 | 262 | 263 | 264 | 265 | 266
        | 269 | 270 | 271 | 272 | 274 | 277 | 278 | 280 | 281 | 282 | 283
        | 284 | 285 | 286 | 287 | 290 | 291 | 292 | 293 | 296 | 297 | 301
        | 305 | 306 | 315 | 316 | 317 | 318 | 319 | 320 | 321 | 322 | 323
        | 332 | 333 | 334 | 336 | 337 | 338 | 339 | 340 | 341 | 342 | 347
        | 512 | 515 | 517 | 518 | 529 | 530 | 531 | 532
        | 33421 | 33422 | 33432 | 33723 | 34377 | 34675
        | 50741 | 50778 | 50779 | 50780 | 50781 | 50879 | 50931 | 50932
        | 50981 | 50982 | 51008 | 51009 | 51022 | 51041
        | 51089 | 51090 | 51091 | 51125)
        || (50706..=50739).contains(&tag)
        || (50827..=50834).contains(&tag)
        || (50934..=50942).contains(&tag)
        || (50964..=50975).contains(&tag)
        || (51107..=51112).contains(&tag)
}

fn exif_value_tag(tag: i64) -> bool {
    matches!(tag,
        33434 | 33437 | 34850 | 34852 | 34855 | 34856 | 34864 | 34865
        | 34866 | 34867 | 34868 | 34869 | 36864 | 36867 | 36868 | 36880
        | 36881 | 36882 | 37121 | 37122 | 37377 | 37378 | 37379 | 37380
        | 37381 | 37382 | 37383 | 37384 | 37385 | 37386 | 37396 | 37510
        | 37520 | 37521 | 37522 | 40960 | 40961 | 40962 | 40963 | 40964
        | 41483 | 41484 | 41486 | 41487 | 41488 | 41492 | 41493 | 41495
        | 41728 | 41729 | 41730 | 41985 | 41986 | 41987 | 41988 | 41989
        | 41990 | 41991 | 41992 | 41993 | 41994 | 41995 | 41996 | 42016
        | 42032 | 42033 | 42034 | 42035 | 42036 | 42037)
}

fn child_kind(kind: Kind, tag: i64) -> Option<Kind> {
    match (kind, tag) {
        (Kind::Image, 330) => Some(Kind::Image),
        (Kind::Image, 34665) => Some(Kind::Exif),
        (Kind::Image, 34853) => Some(Kind::Gps),
        (Kind::Exif, 40965) => Some(Kind::Interop),
        _ => None,
    }
}

// Camera-profile fields are self-contained values, never generic image or
// private pointer fields. The CR container has its own classic TIFF layout.
fn camera_profile_tag(tag: i64) -> bool {
    matches!(tag,
        50708 | 50721 | 50722 | 50725 | 50726 | 50778 | 50779 | 50932
        | 50936 | 50937 | 50938 | 50939 | 50940 | 50941 | 50942 | 50964
        | 50965 | 50981 | 50982 | 51107 | 51108 | 51109 | 51110)
}

fn camera_profile_type(tag: i64, typ: i64) -> bool {
    match tag {
        50708 => typ == 2,
        50932 | 50936 | 50942 => typ == 1 || typ == 2,
        50778 | 50779 => typ == 3,
        50721 | 50722 | 50725 | 50726 | 50964 | 50965 => typ == 10,
        // The specification calls this RATIONAL; the SDK writes SRATIONAL.
        51109 => typ == 5 || typ == 10,
        50937 | 50941 | 50981 | 51107 | 51108 | 51110 => typ == 4,
        50938 | 50939 | 50940 | 50982 => typ == 11,
        _ => false,
    }
}

fn allowed_tag(kind: Kind, tag: i64) -> bool {
    match kind {
        Kind::Image => {
            image_value_tag(tag) || exif_value_tag(tag)
                || [273, 279, 324, 325, 513, 514, 700, 330, 34665, 34853].contains(&tag)
        }
        Kind::Exif => exif_value_tag(tag) || tag == 40965,
        Kind::Gps => (0..=31).contains(&tag),
        Kind::Interop => [1, 2, 4096, 4097, 4098].contains(&tag),
        Kind::CameraProfile => camera_profile_tag(tag),
    }
}

// DNG 1.7 scalar fields observed in SDK-produced JPEG XL containers.
fn extended_image_scalar(tag: i64, typ: i64) -> Option<bool> {
    match tag {
        52547 => Some(typ == 3 || typ == 4), // ColumnInterleaveFactor
        52553 => Some(typ == 11),            // JXLDistance
        52554 | 52555 => Some(typ == 4),     // JXLEffort, JXLDecodeSpeed
        _ => None,
    }
}

fn directory_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "Image",
        Kind::Exif => "Exif",
        Kind::Gps => "GPS",
        Kind::Interop => "Interop",
        Kind::CameraProfile => "CameraProfile",
    }
}

// Simple values: ASCII, SHORT, RATIONAL, SSHORT, SRATIONAL, FLOAT and DOUBLE
// carry only numbers or text.
fn simple_type(typ: i64) -> bool {
    [2, 3, 5, 8, 10, 11, 12].contains(&typ)
}

/// A field admitted at run time: (directory, tag, type).
pub type Admission = (String, i64, i64);

static ADMITTED: RwLock<Vec<Admission>> = RwLock::new(Vec::new());

/// Replace the run-time `merge_only` admissions, with directory Image, Exif,
/// GPS or Interop. Walks read the current list.
pub fn admit_fields(fields: Vec<Admission>) {
    *ADMITTED.write().unwrap_or_else(|e| e.into_inner()) = fields;
}

// OCaml's int_of_string: an optional sign, an optional 0x/0o/0b/0u prefix, and
// digits with underscores after the first.
fn ocaml_int(s: &str) -> Option<i64> {
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = body.to_ascii_lowercase();
    let (radix, digits) = if let Some(d) = lower.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = lower.strip_prefix("0o") {
        (8, d)
    } else if let Some(d) = lower.strip_prefix("0b") {
        (2, d)
    } else if let Some(d) = lower.strip_prefix("0u") {
        (10, d)
    } else {
        (10, lower.as_str())
    };
    if digits.is_empty() || digits.starts_with('_') {
        return None;
    }
    let mut v: i128 = 0;
    for c in digits.chars() {
        if c == '_' {
            continue;
        }
        v = v * radix as i128 + c.to_digit(radix)? as i128;
        if v > (1i128 << 64) {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    i64::try_from(v).ok()
}

/// Parse one `Image|Exif|GPS|Interop:TAG:TYPE` admission; IFD types refused.
pub fn parse_admission(spec: &str) -> Result<Admission, String> {
    let parts: Vec<&str> = spec.split(':').collect();
    match parts.as_slice() {
        [directory, tag, typ] if ["Image", "Exif", "GPS", "Interop"].contains(directory) => {
            match (ocaml_int(tag), ocaml_int(typ)) {
                (Some(tag), Some(typ))
                    if (0..65536).contains(&tag) && (1..=18).contains(&typ) && typ != 13 && typ != 18 =>
                {
                    Ok((directory.to_string(), tag, typ))
                }
                _ => Err(format!("bad tag or type in {spec}")),
            }
        }
        _ => Err(format!("expected Image|Exif|GPS|Interop:TAG:TYPE, got {spec}")),
    }
}

fn merge_only_field(kind: Kind, tag: i64, typ: i64, n: i64) -> Option<bool> {
    match (kind, tag) {
        (Kind::Exif, 37500) => Some(typ == 7 || typ == 1),
        (Kind::Exif, 42082) => Some(typ == 7),
        (Kind::Image, 50740) => Some(typ == 1),
        (Kind::Image, 37393 | 51114) => Some(typ == 4 && n == 1),
        (Kind::Image, 37398) => Some(typ == 1 && n == 4),
        (Kind::Image, 52525 | 52545) => Some(typ == 7),
        (Kind::CameraProfile, _) => None,
        _ if simple_type(typ) => Some(true),
        _ if ADMITTED
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(d, t, y)| d == directory_name(kind) && *t == tag && *y == typ) =>
        {
            Some(true)
        }
        _ => None,
    }
}

fn number<E>(little: bool, s: &[u8], pos: usize, n: usize) -> Step<i64, E> {
    let mut v: u64 = 0;
    for j in 0..n {
        let i = if little { n - 1 - j } else { j };
        let Some(&byte) = s.get(pos + i) else { return reject("short read") };
        v = (v << 8) | byte as u64;
    }
    // File positions and sizes are signed 64-bit throughout. Reject unsigned
    // encodings outside that domain before arithmetic.
    if v > i64::MAX as u64 {
        return reject("unsigned TIFF integer exceeds signed int64");
    }
    Ok(v as i64)
}

/// Optional observers of a walk: `record` sees each physical region (and may
/// refuse, ending the walk); `unknown` turns an unknown field from a refusal
/// into a note of (directory, tag, type, count).
pub type RecordFn<'a> = &'a mut dyn FnMut(&ComponentRegion) -> Result<(), String>;
/// (directory, tag, type, count) of a field outside the grammar.
pub type UnknownFn<'a> = &'a mut dyn FnMut(&str, i64, i64, i64);

#[derive(Default)]
pub struct Observers<'a> {
    pub record: Option<RecordFn<'a>>,
    pub unknown: Option<UnknownFn<'a>>,
}

/// Walk the file under `policy`. `Ok(Some(packet))` for the primary XMP packet,
/// `Ok(None)` only when the policy does not require one.
pub fn walk<R: Reader>(policy: &Policy, reader: &mut R, obs: Observers<'_>) -> Step<Option<Xmp>, R::Error> {
    let Policy { extended, opaque_maker, layout_only, require_xmp, max_xmp } = *policy;
    let Observers { mut record, mut unknown } = obs;
    let size = reader.size();
    let read = |r: &mut R, off: i64, len: usize| -> Step<Vec<u8>, R::Error> {
        let d = r.read(off, len).map_err(WalkError::Reader)?;
        if d.len() != len {
            return reject("short read");
        }
        Ok(d)
    };
    let byte_order = |header: &[u8]| -> Step<bool, R::Error> {
        match &header[0..2] {
            b"II" => Ok(true),
            b"MM" => Ok(false),
            _ => reject("not TIFF"),
        }
    };
    let header = read(reader, 0, 8)?;
    let little = byte_order(&header)?;
    let (layout, header_size, primary) = match number(little, &header, 2, 2)? {
        42 => (CLASSIC, 8i64, number(little, &header, 4, 4)?),
        43 if extended => {
            if number(little, &header, 4, 2)? != 8 || number(little, &header, 6, 2)? != 0 {
                return reject("invalid BigTIFF header");
            }
            let header = read(reader, 0, 16)?;
            (BIGTIFF, 16, number(little, &header, 8, 8)?)
        }
        _ => return reject("unsupported TIFF version (including BigTIFF)"),
    };
    let extent = |offset: i64, length: i64| -> Step<(i64, i64), R::Error> {
        if offset < 0 || length < 0 || offset > size || length > size - offset {
            return reject("TIFF extent outside file");
        }
        Ok((offset, offset + length))
    };
    let relative = |base: i64, offset: i64| -> Step<i64, R::Error> {
        if base < 0 || base > size || offset < 0 || offset > size - base {
            return reject("TIFF relative offset outside file");
        }
        Ok(base + offset)
    };
    let mut protected: Vec<(i64, i64)> = vec![extent(0, header_size)?];
    let mut extent_count = 1usize;
    let mut protect = |protected: &mut Vec<(i64, i64)>, offset: i64, length: i64| -> Step<(), R::Error> {
        extent_count += 1;
        if extent_count > 131072 {
            return reject("protected extent limit");
        }
        protected.push(extent(offset, length)?);
        Ok(())
    };
    let mut record_region = |schema: &str, logical_id: String, role: ComponentRole, offset: i64, length: i64| -> Step<(), R::Error> {
        if length > 0 && let Some(r) = record.as_mut() {
            r(&ComponentRegion { logical_id, schema: schema.to_string(), role, offset, length })
                .map_err(WalkError::Refused)?;
        }
        Ok(())
    };
    const STRUCTURE: &str = "tiff-structure/0";
    record_region(STRUCTURE, "header".into(), ComponentRole::Structure, 0, header_size)?;
    let mut directory_index = 0usize;
    let mut camera_header_index = 0usize;
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<Pending> = VecDeque::new();
    // Camera headers and directories share the same visited/pending ceiling.
    let queue_room = |queue: &VecDeque<Pending>, seen: &HashSet<i64>| -> Step<(), R::Error> {
        if queue.len() + seen.len() >= 256 {
            return reject("IFD work limit");
        }
        Ok(())
    };
    let enqueue = |queue: &mut VecDeque<Pending>, seen: &HashSet<i64>, kind: Kind, little: bool, layout: Layout, base: i64, offset: i64| -> Step<(), R::Error> {
        if offset != 0 {
            queue_room(queue, seen)?;
            let offset = relative(base, offset)?;
            queue.push_back(Pending::Directory(Directory { kind, little, layout, base, offset }));
        }
        Ok(())
    };
    let enqueue_camera = |queue: &mut VecDeque<Pending>, seen: &HashSet<i64>, offset: i64| -> Step<(), R::Error> {
        if offset == 0 {
            return reject("zero camera profile offset");
        }
        queue_room(queue, seen)?;
        extent(offset, 8)?;
        queue.push_back(Pending::CameraHeader(offset));
        Ok(())
    };
    let visit = |seen: &mut HashSet<i64>, offset: i64| -> Step<(), R::Error> {
        if seen.contains(&offset) {
            return reject("repeated or cyclic IFD");
        }
        if seen.len() >= 256 {
            return reject("IFD limit");
        }
        seen.insert(offset);
        Ok(())
    };
    enqueue(&mut queue, &seen, Kind::Image, little, layout, 0, primary)?;
    let mut entries = 0i64;
    let mut xmp: Option<Xmp> = None;
    let mut dng = false;
    while let Some(pending) = queue.pop_front() {
        match pending {
            Pending::CameraHeader(base) => {
                visit(&mut seen, base)?;
                protect(&mut protected, base, 8)?;
                record_region(STRUCTURE, format!("camera-header/{camera_header_index}"), ComponentRole::Structure, base, 8)?;
                camera_header_index += 1;
                let header = read(reader, base, 8)?;
                let little = byte_order(&header)?;
                if number(little, &header, 2, 2)? != 0x4352 {
                    return reject("invalid camera profile header");
                }
                let offset = number(little, &header, 4, 4)?;
                if offset < 8 {
                    return reject("invalid camera profile IFD offset");
                }
                enqueue(&mut queue, &seen, Kind::CameraProfile, little, CLASSIC, base, offset)?;
            }
            Pending::Directory(Directory { kind, little, layout, base, offset }) => {
                let num = |s: &[u8], pos: usize, n: usize| number::<R::Error>(little, s, pos, n);
                let logical_directory = format!("ifd/{directory_index}");
                directory_index += 1;
                visit(&mut seen, offset)?;
                let count64 = num(&read(reader, offset, layout.count_width)?, 0, layout.count_width)?;
                if count64 > 16384 - entries {
                    return reject("TIFF entry limit");
                }
                let count = count64 as usize;
                if kind == Kind::CameraProfile && count == 0 {
                    return reject("empty camera profile IFD");
                }
                entries += count64;
                let length = layout.count_width + layout.entry_size * count + layout.number_width;
                protect(&mut protected, offset, length as i64)?;
                record_region(STRUCTURE, format!("{logical_directory}/count"), ComponentRole::Structure, offset, layout.count_width as i64)?;
                record_region(STRUCTURE, format!("{logical_directory}/next"), ComponentRole::Structure,
                    offset + (layout.count_width + layout.entry_size * count) as i64, layout.number_width as i64)?;
                let directory = read(reader, offset, length)?;
                let mut tags: HashMap<i64, (i64, i64, i64)> = HashMap::with_capacity(count);
                for i in 0..count {
                    let at = layout.count_width + layout.entry_size * i;
                    let tag = num(&directory, at, 2)?;
                    let typ = num(&directory, at + 2, 2)?;
                    let n = num(&directory, at + 4, layout.number_width)?;
                    if tags.contains_key(&tag) {
                        return reject("duplicate TIFF tag");
                    }
                    let profile_pointer = extended && kind == Kind::Image && tag == 50933;
                    let scalar = if extended && kind == Kind::Image { extended_image_scalar(tag, typ) } else { None };
                    if let Some(valid) = scalar && (!valid || n != 1) {
                        return reject("invalid extended DNG scalar type or count");
                    }
                    let merge_only = opaque_maker && merge_only_field(kind, tag, typ, n).is_some();
                    if merge_only && merge_only_field(kind, tag, typ, n) == Some(false) {
                        return reject("invalid merge-only field type or count");
                    }
                    if merge_only && tag == 50740 && offset != primary {
                        return reject("DNGPrivateData outside primary IFD");
                    }
                    if !(layout_only || allowed_tag(kind, tag) || profile_pointer || scalar.is_some() || merge_only) {
                        match unknown.as_mut() {
                            Some(note) => note(directory_name(kind), tag, typ, n),
                            None => return Err(WalkError::Refused(format!("unsupported TIFF tag {tag} (opaque or private graph)"))),
                        }
                    }
                    let child = child_kind(kind, tag);
                    if !layout_only && (typ == 13 || (extended && typ == 18)) && child.is_none() {
                        return reject("IFD type on non-pointer field");
                    }
                    if kind == Kind::CameraProfile && (!layout_only || allowed_tag(kind, tag)) && !camera_profile_type(tag, typ) {
                        return reject("unsupported camera profile field type");
                    }
                    let width: i64 = match typ {
                        1 | 2 | 6 | 7 => 1,
                        3 | 8 => 2,
                        4 | 9 | 11 | 13 => 4,
                        5 | 10 | 12 => 8,
                        16..=18 if layout.big => 8,
                        _ => return reject("unsupported TIFF field type"),
                    };
                    if n > i64::MAX / width {
                        return reject("TIFF field size overflow");
                    }
                    let bytes = n * width;
                    let value_offset = if bytes <= layout.inline_width as i64 {
                        offset + (at + layout.value_at) as i64
                    } else {
                        relative(base, num(&directory, at + layout.value_at, layout.number_width)?)?
                    };
                    extent(value_offset, bytes)?;
                    tags.insert(tag, (typ, n, value_offset));
                    let logical_entry = format!("{logical_directory}/tag/{tag}");
                    let value_role = if allowed_tag(kind, tag) || profile_pointer || scalar.is_some() {
                        ComponentRole::StandardValue
                    } else {
                        ComponentRole::PrivateValue
                    };
                    record_region(STRUCTURE, format!("{logical_entry}/descriptor"), ComponentRole::Structure,
                        offset + at as i64, layout.value_at as i64)?;
                    let schema = format!("tiff-value/0:{}:{}:{}", if little { "II" } else { "MM" }, typ, n);
                    record_region(&schema, format!("{logical_entry}/value"), value_role, value_offset, bytes)?;
                    let inline_start = offset + (at + layout.value_at) as i64;
                    if bytes > layout.inline_width as i64 {
                        record_region(STRUCTURE, format!("{logical_entry}/offset"), ComponentRole::Structure,
                            inline_start, layout.inline_width as i64)?;
                    } else {
                        record_region(STRUCTURE, format!("{logical_entry}/padding"), ComponentRole::Structure,
                            inline_start + bytes, layout.inline_width as i64 - bytes)?;
                    }
                    // In merge_only an auxiliary image's own packet, such as a
                    // portrait matte's, is an ordinary protected value; only the
                    // primary IFD's packet is located.
                    if tag == 700 && !(opaque_maker && offset != primary) {
                        if xmp.is_some() {
                            return reject("multiple XMP packets");
                        }
                        // Lightroom mobile writes the primary packet as UNDEFINED; only merge_only takes it.
                        if !(typ == 1 || (opaque_maker && typ == 7)) || n <= layout.inline_width as i64 || n > max_xmp as i64 {
                            return reject("unsupported XMP type or packet size");
                        }
                        xmp = Some(Xmp { offset: value_offset, length: n as usize });
                    } else if bytes > layout.inline_width as i64 {
                        protect(&mut protected, value_offset, bytes)?;
                    }
                    if tag == 50706 {
                        if typ != 1 || n != 4 {
                            return reject("invalid DNGVersion");
                        }
                        let version = read(reader, value_offset, 4)?;
                        if version[0] != 1 || version[1] > 7 {
                            return reject("unsupported DNG version");
                        }
                        if kind == Kind::Image && offset == primary {
                            dng = true;
                        }
                    }
                    if let Some(child_kind) = child {
                        let pointer_width = match typ {
                            4 | 13 => 4,
                            16 | 18 if layout.big => 8,
                            _ => return reject("invalid IFD pointer type"),
                        };
                        if tag != 330 && n != 1 {
                            return reject("invalid singleton IFD pointer count");
                        }
                        if n > 256 {
                            return reject("IFD pointer limit");
                        }
                        let pointers = read(reader, value_offset, pointer_width * n as usize)?;
                        for j in 0..n as usize {
                            let p = num(&pointers, pointer_width * j, pointer_width)?;
                            enqueue(&mut queue, &seen, child_kind, little, layout, base, p)?;
                        }
                    }
                    if profile_pointer {
                        if offset != primary {
                            return reject("camera profile list outside primary IFD");
                        }
                        let pointer_width = match typ {
                            4 => 4,
                            16 if layout.big => 8,
                            _ => return reject("invalid camera profile pointer type"),
                        };
                        if n == 0 || n > 256 {
                            return reject("camera profile pointer limit");
                        }
                        let pointers = read(reader, value_offset, pointer_width * n as usize)?;
                        for j in 0..n as usize {
                            let p = num(&pointers, pointer_width * j, pointer_width)?;
                            enqueue_camera(&mut queue, &seen, p)?;
                        }
                    }
                }
                for (offsets, lengths) in [(273, 279), (324, 325), (513, 514)] {
                    let has_o = tags.contains_key(&offsets);
                    let has_l = tags.contains_key(&lengths);
                    if has_o != has_l {
                        return reject("unpaired image extents");
                    }
                    if has_o {
                        let mut values = |tag: i64| -> Step<(i64, usize, Vec<u8>), R::Error> {
                            let (typ, n, off) = tags[&tag];
                            if n > 65536 {
                                return reject("image extent limit");
                            }
                            let width = match typ {
                                3 => 2,
                                4 => 4,
                                16 if layout.big => 8,
                                _ => return reject("invalid image extent type"),
                            };
                            Ok((n, width, read(reader, off, n as usize * width)?))
                        };
                        let (n, wo, data_o) = values(offsets)?;
                        let (m, wl, data_l) = values(lengths)?;
                        if n != m {
                            return reject("mismatched image extents");
                        }
                        for i in 0..n as usize {
                            let start = relative(base, num(&data_o, i * wo, wo)?)?;
                            let length = num(&data_l, i * wl, wl)?;
                            protect(&mut protected, start, length)?;
                            if kind == Kind::Image {
                                record_region("tiff-encoded-unit/0", format!("{logical_directory}/image/{offsets}/{i}"),
                                    ComponentRole::ImageBytes, start, length)?;
                            }
                        }
                    }
                }
                let next = num(&directory, layout.count_width + layout.entry_size * count, layout.number_width)?;
                if kind == Kind::CameraProfile && next != 0 {
                    return reject("unsupported camera profile IFD chain");
                }
                enqueue(&mut queue, &seen, kind, little, layout, base, next)?;
            }
        }
    }
    if !dng {
        return reject("missing DNGVersion");
    }
    match xmp {
        None if require_xmp => return reject("no embedded XMP"),
        None => {}
        Some(region) => {
            let stop = region.offset + region.length as i64;
            // OCaml checks the protected list newest first; any hit gives the same reason.
            if protected.iter().any(|&(a, b)| a < stop && region.offset < b) {
                return reject("XMP aliases protected extent");
            }
        }
    }
    reader.check().map_err(WalkError::Reader)?;
    Ok(xmp)
}

/// Survey only: the `merge_only` walk, noting each field outside its grammar
/// instead of stopping at the first, plus the first other reason the walk
/// stopped. Reader errors are returned as such.
#[allow(clippy::type_complexity)]
pub fn unknown_fields<R: Reader>(max_xmp: usize, reader: &mut R) -> Result<(Vec<(String, i64, i64, i64)>, Option<String>), R::Error> {
    let mut found = Vec::new();
    let mut note = |d: &str, tag: i64, typ: i64, n: i64| found.push((d.to_string(), tag, typ, n));
    let stop = match walk(&Policy::merge_only(max_xmp), reader, Observers { record: None, unknown: Some(&mut note) }) {
        Ok(_) => None,
        Err(WalkError::Refused(reason)) => Some(reason),
        Err(WalkError::Reader(e)) => return Err(e),
    };
    Ok((found, stop))
}

/// At most 65,536 advisory physical claims from the `layout` walk, with no XMP
/// requirement. Aliases are not independent components.
pub fn component_regions<R: Reader>(max_xmp: usize, reader: &mut R) -> Step<Vec<ComponentRegion>, R::Error> {
    let mut regions = Vec::new();
    let mut record = |r: &ComponentRegion| -> Result<(), String> {
        if regions.len() >= 65536 {
            return Err("DNG raw component region limit".to_string());
        }
        regions.push(r.clone());
        Ok(())
    };
    let policy = Policy { require_xmp: false, ..Policy::layout(max_xmp) };
    walk(&policy, reader, Observers { record: Some(&mut record), unknown: None })?;
    Ok(regions)
}
