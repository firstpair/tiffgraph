//! `cargo run --example dump -- FILE...`: what every grammar makes of each
//! file, in the same text form as the OCaml build's test/dump.exe, so
//! test/cross.py can compare the two builds byte for byte.
use tiffgraph::{ComponentRole, Observers, Policy, SliceReader, WalkError};

const MAX_XMP: usize = 1 << 20;

fn role(r: ComponentRole) -> &'static str {
    match r {
        ComponentRole::ImageBytes => "image",
        ComponentRole::StandardValue => "standard",
        ComponentRole::PrivateValue => "private",
        ComponentRole::Structure => "structure",
    }
}

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap_or_default();
        println!("file\t{path}");
        for (name, policy) in [("v0", Policy::v0(MAX_XMP)), ("v1", Policy::v1(MAX_XMP)),
                               ("merge_only", Policy::merge_only(MAX_XMP)), ("layout", Policy::layout(MAX_XMP))] {
            match tiffgraph::walk(&policy, &mut SliceReader(&bytes), Observers::default()) {
                Ok(Some(x)) => println!("{name}\tok\t{}\t{}", x.offset, x.length),
                Ok(None) => println!("{name}\tnone"),
                Err(WalkError::Refused(r)) => println!("{name}\trefused\t{r}"),
                Err(WalkError::Reader(_)) => println!("{name}\treader"),
            }
        }
        match tiffgraph::unknown_fields(MAX_XMP, &mut SliceReader(&bytes)) {
            Ok((fields, stop)) => {
                for (d, tag, typ, n) in fields {
                    println!("unknown\t{d}\t{tag}\t{typ}\t{n}");
                }
                println!("unknown-stop\t{}", stop.unwrap_or_default());
            }
            Err(_) => println!("unknown-stop\treader"),
        }
        match tiffgraph::component_regions(MAX_XMP, &mut SliceReader(&bytes)) {
            Ok(regions) => {
                for r in regions {
                    println!("region\t{}\t{}\t{}\t{}\t{}", r.logical_id, r.schema, role(r.role), r.offset, r.length);
                }
                println!("regions\tok");
            }
            Err(WalkError::Refused(r)) => println!("regions\trefused\t{r}"),
            Err(WalkError::Reader(_)) => println!("regions\treader"),
        }
    }
}
