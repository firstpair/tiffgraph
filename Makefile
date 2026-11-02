# DUNE may point at a switch, e.g. DUNE="opam exec --switch 5.2.0+ox -- dune".
DUNE ?= dune
.PHONY: build test both clean
build:
	$(DUNE) build @install
test:
	$(DUNE) test --force
both:
	opam exec --switch default -- dune test --force
	opam exec --switch 5.2.0+ox -- dune test --force
clean:
	$(DUNE) clean
