# DUNE may point at a switch, e.g. DUNE="opam exec --switch 5.2.0+ox -- dune".
DUNE ?= opam exec -- dune
.PHONY: build test both rust cross all clean
build:
	$(DUNE) build @install
test:
	$(DUNE) test --force
both:
	opam exec --switch default -- dune test --force
	opam exec --switch 5.2.0+ox -- dune test --force
rust:
	cd rust && cargo test --release && cargo clippy --release --all-targets -- -D warnings
# The OCaml and Rust builds must agree on every file (test/cross.py).
cross:
	$(DUNE) build test/dump.exe
	cd rust && cargo build --release --examples
	python3 test/cross.py --ocaml _build/default/test/dump.exe --rust rust/target/release/examples/dump
all: both rust cross
clean:
	$(DUNE) clean
	cd rust && cargo clean
