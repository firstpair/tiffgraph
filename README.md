# tiffgraph

A bounded TIFF/DNG graph walker for OCaml, OxCaml and Rust, with no dependencies.

It finds a DNG's primary XMP packet under a fixed grammar, protects every other
declared extent so the packet may not alias it, and reports the file's
physical regions: header, directory counts and descriptors, values, padding,
and encoded image bytes. Anything outside the grammar is refused with a
reason, never guessed.

It is not a DNG validator, a decoder or a file identity. A caller that rewrites
the XMP packet must still prove separately that every byte outside it is kept.
For named file identities, including DNG raw blocks, see
[phidentify](https://github.com/firstpair/phidentify).

## Grammars

| Policy | Admits |
| --- | --- |
| `v0` | classic TIFF, a fixed set of standard Image, Exif, GPS and Interop tags, one BYTE XMP packet |
| `v1` | `v0` plus BigTIFF, bounded embedded camera-profile graphs and DNG 1.7 JPEG XL scalars |
| `merge_only` | `v1` plus MakerNote, primary-IFD DNGPrivateData, named scalars, any field of a simple numeric or text type, run-time admissions, and an UNDEFINED primary packet; all kept as protected values |
| `layout` | `v1` geometry with unknown fields protected byte-exact; no merge authority |

These are the grammars Semantic Unison negotiates as its
`org.firstpair.dng-xmp/0`, `/1` and `/2` profiles. The walker began as its
`DngSemantic` module. Changing what a grammar admits changes those profiles.

Every walk is bounded: 256 directories, 16,384 entries, 131,072 protected
extents, 65,536 component regions, and the `max_xmp` the caller sets.

## Use

```ocaml
let reader = { Tiffgraph.size; read; check } in
match Tiffgraph.walk (Tiffgraph.v1 ~max_xmp:(1 lsl 20)) reader with
| Ok (Some { Tiffgraph.offset; length }) -> (* the packet's extent *) ()
| Ok None -> (* only when the policy does not require a packet *) ()
| Error reason -> prerr_endline reason
```

`read offset length` returns exactly `length` bytes or raises; its exceptions
propagate unchanged. `check` runs once after a successful walk, for example to
confirm the file did not change while it was read. `Tiffgraph.unknown_fields`
surveys a file for fields outside the `merge_only` grammar, and
`Tiffgraph.component_regions` lists its physical regions.

## Rust

The `rust/` crate, `tiffgraph`, is a line-for-line port of the OCaml module:
the same grammars, limits, order of checks and refusal reasons.

```rust
use tiffgraph::{walk, FileReader, Observers, Policy};
let mut reader = FileReader::open(path)?;
match walk(&Policy::v1(1 << 20), &mut reader, Observers::default()) {
    Ok(Some(xmp)) => println!("XMP at {}, {} bytes", xmp.offset, xmp.length),
    Ok(None) => {}
    Err(e) => eprintln!("refused: {e}"),
}
```

A `Reader` is `size`, `read` and an optional `check`; `SliceReader` and
`FileReader` are provided. Reader errors come back as `WalkError::Reader`,
refusals as `WalkError::Refused`.

`make cross` runs both builds on every file of `test/cross.py`, synthetic DNGs
and a few thousand mutations of them, and fails on any difference in packets,
refusal reasons, unknown-field surveys or component regions. `make all` runs
the OCaml tests in both switches, the Rust tests and clippy, and the cross
check.

## Manual

The manual is on the Photography shelf at
[firstpair.org/books/tiffgraph](https://firstpair.org/books/tiffgraph/); its source is
`docs/tiffgraph-manual.md`.

## Build and test

```sh
dune build
make both      # tests on the default (stock OCaml 5) and 5.2.0+ox (OxCaml) switches
make rust      # the Rust crate's tests and clippy
make cross     # OCaml and Rust agree on every file
```

The tests build every file in memory from TIFF entries and need no fixtures:
both byte orders, classic TIFF and BigTIFF, each grammar's acceptances and
refusals, extents, aliasing, cycles, admissions and component regions. Under
OxCaml the interface is `@@ portable`.

## Licence

MIT.
