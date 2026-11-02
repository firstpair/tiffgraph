//! Synthetic controls for the Rust build: every file is built here, in memory,
//! from TIFF entries. test/cross.py checks the Rust build against the OCaml
//! one file by file; these check behaviour directly, including the limits no
//! ordinary file reaches.
use tiffgraph::*;

const MAX: usize = 65536;

enum V {
    Data(u16, u32, Vec<u8>),
    Strip,
    Long(u32),
    Child(Vec<(u16, V)>),
}

struct Built {
    bytes: Vec<u8>,
    xmp_at: Option<i64>,
    strip_at: i64,
}

fn build(little: bool, big: bool, ifd0: Vec<(u16, V)>, strip: &[u8]) -> Built {
    let enc = |v: u64, w: usize| -> Vec<u8> {
        let b = v.to_be_bytes();
        let mut x = b[8 - w..].to_vec();
        if little {
            x.reverse();
        }
        x
    };
    let (cw, nw, inline) = if big { (8, 8, 8) } else { (2, 4, 4) };
    let mut out: Vec<u8> = if little { b"II".to_vec() } else { b"MM".to_vec() };
    out.extend(enc(if big { 43 } else { 42 }, 2));
    if big {
        out.extend(enc(8, 2));
        out.extend(enc(0, 2));
    }
    let first = out.len();
    out.extend(vec![0; nw]);
    let mut patches: Vec<(usize, Vec<u8>)> = vec![];
    let mut strip_slots = vec![];
    let mut xmp_at = None;
    #[allow(clippy::too_many_arguments)]
    fn ifd(out: &mut Vec<u8>, items: Vec<(u16, V)>, enc: &dyn Fn(u64, usize) -> Vec<u8>, cw: usize, nw: usize,
           inline: usize, patches: &mut Vec<(usize, Vec<u8>)>, strips: &mut Vec<usize>, xmp_at: &mut Option<i64>) -> usize {
        let at = out.len();
        out.extend(enc(items.len() as u64, cw));
        let mut slots = vec![];
        for (tag, v) in &items {
            let (typ, count) = match v {
                V::Data(t, c, _) => (*t, *c as u64),
                _ => (4, 1),
            };
            out.extend(enc(*tag as u64, 2));
            out.extend(enc(typ as u64, 2));
            out.extend(enc(count, nw));
            slots.push(out.len());
            out.extend(vec![0; inline]);
        }
        out.extend(vec![0; nw]);
        let mut children = vec![];
        for ((tag, v), slot) in items.into_iter().zip(slots) {
            match v {
                V::Data(_, _, d) if d.len() <= inline => patches.push((slot, d)),
                V::Data(_, _, d) => {
                    patches.push((slot, enc(out.len() as u64, nw)));
                    if tag == 700 && xmp_at.is_none() {
                        *xmp_at = Some(out.len() as i64);
                    }
                    out.extend(d);
                }
                V::Strip => strips.push(slot),
                V::Long(x) => patches.push((slot, enc(x as u64, 4))),
                V::Child(c) => children.push((slot, c)),
            }
        }
        for (slot, c) in children {
            let child = ifd(out, c, enc, cw, nw, inline, patches, strips, xmp_at);
            patches.push((slot, enc(child as u64, 4)));
        }
        at
    }
    let i0 = ifd(&mut out, ifd0, &enc, cw, nw, inline, &mut patches, &mut strip_slots, &mut xmp_at);
    patches.push((first, enc(i0 as u64, nw)));
    let strip_at = out.len();
    out.extend_from_slice(strip);
    for s in strip_slots {
        patches.push((s, enc(strip_at as u64, 4)));
    }
    for (at, d) in patches {
        out[at..at + d.len()].copy_from_slice(&d);
    }
    Built { bytes: out, xmp_at, strip_at: strip_at as i64 }
}

fn long(v: u32, little: bool) -> V {
    V::Data(4, 1, if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() })
}
fn xmp(n: usize, typ: u16) -> (u16, V) {
    (700, V::Data(typ, n as u32, b"<x:xmpmeta/>".iter().cycle().take(n).copied().collect()))
}
fn dng(little: bool, extra: Vec<(u16, V)>, with_xmp: bool) -> Vec<(u16, V)> {
    let mut v = vec![(254, long(0, little)), (273, V::Strip), (279, long(32, little)),
                     (50706, V::Data(1, 4, vec![1, 4, 0, 0]))];
    if with_xmp {
        v.push(xmp(100, 1));
    }
    v.extend(extra);
    v
}
fn plain(little: bool, big: bool, extra: Vec<(u16, V)>) -> Built {
    build(little, big, dng(little, extra, true), &[0xaa; 32])
}

fn walk_b(p: Policy, b: &Built) -> Result<Option<Xmp>, WalkError<String>> {
    walk(&p, &mut SliceReader(&b.bytes), Observers::default())
}
fn refused(p: Policy, b: &Built, reason: &str) -> bool {
    matches!(walk_b(p, b), Err(WalkError::Refused(r)) if r.contains(reason))
}
fn found(p: Policy, b: &Built) -> bool {
    matches!(walk_b(p, b), Ok(Some(x)) if Some(x.offset) == b.xmp_at && x.length == 100)
}

#[test]
fn grammars_accept_minimal_dngs() {
    for little in [true, false] {
        let b = plain(little, false, vec![]);
        for p in [Policy::v0(MAX), Policy::v1(MAX), Policy::merge_only(MAX), Policy::layout(MAX)] {
            assert!(found(p, &b), "minimal DNG, little={little}");
        }
        let big = plain(little, true, vec![]);
        assert!(refused(Policy::v0(MAX), &big, "unsupported TIFF version"));
        assert!(found(Policy::v1(MAX), &big), "BigTIFF under v1, little={little}");
    }
}

#[test]
fn refusals() {
    let no_xmp = build(true, false, dng(true, vec![], false), &[0; 32]);
    assert!(refused(Policy::v0(MAX), &no_xmp, "no embedded XMP"));
    assert!(matches!(walk_b(Policy { require_xmp: false, ..Policy::v1(MAX) }, &no_xmp), Ok(None)));
    let no_version = build(true, false, dng(true, vec![], true).into_iter().filter(|(t, _)| *t != 50706).collect(), &[0; 32]);
    assert!(refused(Policy::v0(MAX), &no_version, "missing DNGVersion"));
    assert!(refused(Policy::v0(MAX), &plain(true, false, vec![(254, long(1, true))]), "duplicate TIFF tag"));
    assert!(refused(Policy::v0(MAX), &plain(true, false, vec![(34665, V::Long(8))]), "repeated or cyclic IFD"));
    let unpaired = build(true, false, dng(true, vec![], true).into_iter().filter(|(t, _)| *t != 279).collect(), &[0; 32]);
    assert!(refused(Policy::v0(MAX), &unpaired, "unpaired image extents"));
    let two = plain(true, false, vec![(330, V::Child(vec![(254, long(1, true)), xmp(50, 1)]))]);
    assert!(refused(Policy::v0(MAX), &two, "multiple XMP packets"));
    let mut cut = plain(true, false, vec![]);
    cut.bytes.truncate(cut.xmp_at.unwrap() as usize + 50);
    assert!(refused(Policy::v0(MAX), &cut, "outside file"));
    let probe = plain(true, false, vec![]);
    let aliased = build(true, false, dng(true, vec![], true).into_iter()
        .map(|(t, v)| if t == 273 { (t, V::Long(probe.xmp_at.unwrap() as u32)) } else { (t, v) }).collect(), &[0; 32]);
    assert!(refused(Policy::v0(MAX), &aliased, "XMP aliases protected extent"));
    assert!(refused(Policy::v0(99), &plain(true, false, vec![]), "unsupported XMP type or packet size"));
    let not_tiff = { let mut b = plain(true, false, vec![]); b.bytes[0] = b'P'; b.bytes[1] = b'K'; b };
    assert!(refused(Policy::v0(MAX), &not_tiff, "not TIFF"));
}

#[test]
fn widening_and_admissions() {
    let undefined = build(true, false, dng(true, vec![], false).into_iter().chain([xmp(100, 7)]).collect(), &[0; 32]);
    assert!(refused(Policy::v1(MAX), &undefined, "unsupported XMP type"));
    assert!(found(Policy::merge_only(MAX), &undefined));
    let unknown_long = plain(true, false, vec![(65001, long(7, true))]);
    assert!(refused(Policy::merge_only(MAX), &unknown_long, "unsupported TIFF tag 65001"));
    assert!(found(Policy::layout(MAX), &unknown_long));
    let unknown_short = plain(true, false, vec![(65002, V::Data(3, 1, vec![7, 0]))]);
    assert!(refused(Policy::v1(MAX), &unknown_short, "unsupported TIFF tag 65002"));
    assert!(found(Policy::merge_only(MAX), &unknown_short));
    admit_fields(vec![("Image".into(), 65001, 4)]);
    assert!(found(Policy::merge_only(MAX), &unknown_long));
    admit_fields(vec![]);
    assert!(refused(Policy::merge_only(MAX), &unknown_long, "unsupported TIFF tag 65001"));
    assert_eq!(parse_admission("Exif:37500:7"), Ok(("Exif".into(), 37500, 7)));
    assert_eq!(parse_admission("Exif:0x927c:7"), Ok(("Exif".into(), 37500, 7)));
    assert!(parse_admission("Image:65001:13").is_err());
    assert!(parse_admission("CameraProfile:50708:2").is_err());
    let maker = plain(true, false, vec![(34665, V::Child(vec![(37500, V::Data(7, 15, b"MakerNote-bytes".to_vec()))]))]);
    assert!(refused(Policy::v1(MAX), &maker, "unsupported TIFF tag 37500"));
    assert!(found(Policy::merge_only(MAX), &maker));
    let survey = plain(true, false, vec![(65001, long(7, true)), (65003, V::Data(7, 8, b"abcdefgh".to_vec()))]);
    let (fields, stop) = unknown_fields(MAX, &mut SliceReader(&survey.bytes)).unwrap();
    assert_eq!(fields, vec![("Image".to_string(), 65001, 4, 1), ("Image".to_string(), 65003, 7, 8)]);
    assert_eq!(stop, None);
}

#[test]
fn regions_and_reader_contract() {
    let b = build(true, false, dng(true, vec![(65001, long(7, true))], false), &[0xaa; 32]);
    let regions = component_regions(MAX, &mut SliceReader(&b.bytes)).unwrap();
    let has = |f: &dyn Fn(&ComponentRegion) -> bool| regions.iter().any(f);
    assert!(has(&|r| r.logical_id == "header" && r.role == ComponentRole::Structure && r.length == 8));
    assert!(has(&|r| r.role == ComponentRole::ImageBytes && r.offset == b.strip_at && r.length == 32));
    assert!(has(&|r| r.logical_id == "ifd/0/tag/65001/value" && r.role == ComponentRole::PrivateValue));
    assert!(regions.iter().all(|r| r.offset >= 0 && r.offset + r.length <= b.bytes.len() as i64));

    struct Counting<'a>(SliceReader<'a>, usize, bool);
    impl Reader for Counting<'_> {
        type Error = String;
        fn size(&self) -> i64 { self.0.size() }
        fn read(&mut self, o: i64, n: usize) -> Result<Vec<u8>, String> {
            if self.2 { Err("reader failed".into()) } else { self.0.read(o, n) }
        }
        fn check(&mut self) -> Result<(), String> { self.1 += 1; Ok(()) }
    }
    let ok = plain(true, false, vec![]);
    let mut r = Counting(SliceReader(&ok.bytes), 0, false);
    walk(&Policy::v0(MAX), &mut r, Observers::default()).unwrap();
    assert_eq!(r.1, 1, "check runs once after success");
    let mut r = Counting(SliceReader(&no_xmp_bytes()), 0, false);
    assert!(walk(&Policy::v0(MAX), &mut r, Observers::default()).is_err());
    assert_eq!(r.1, 0, "check never runs after a refusal");
    let mut r = Counting(SliceReader(&ok.bytes), 0, true);
    assert_eq!(walk(&Policy::v0(MAX), &mut r, Observers::default()), Err(WalkError::Reader("reader failed".into())));
}

fn no_xmp_bytes() -> Vec<u8> {
    build(true, false, dng(true, vec![], false), &[0; 32]).bytes
}

#[test]
fn limits() {
    // More strip pieces than the walk will follow.
    let n = 65537u32;
    let offsets: Vec<u8> = (0..n).flat_map(|_| 8u32.to_le_bytes()).collect();
    let counts: Vec<u8> = (0..n).flat_map(|_| 0u32.to_le_bytes()).collect();
    let b = build(true, false, vec![(254, long(0, true)), (273, V::Data(4, n, offsets)), (279, V::Data(4, n, counts)),
                                     (50706, V::Data(1, 4, vec![1, 4, 0, 0])), xmp(100, 1)], &[]);
    assert!(refused(Policy::v0(MAX), &b, "image extent limit"));
    // A count of entries past the walk's bound.
    let mut e = plain(true, false, vec![]);
    let at = u32::from_le_bytes(e.bytes[4..8].try_into().unwrap()) as usize;
    e.bytes[at..at + 2].copy_from_slice(&16385u16.to_le_bytes());
    assert!(refused(Policy::v0(MAX), &e, "TIFF entry limit"));
    // An 8-byte number past signed 64 bits.
    let mut big = plain(true, true, vec![]);
    big.bytes[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(refused(Policy::v1(MAX), &big, "unsigned TIFF integer exceeds signed int64"));
}
