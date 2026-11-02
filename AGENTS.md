# tiffgraph: agent notes

Shared rules for every repository under `~/src` are in `~/src/firstpair/AGENTS.md`.
This file only adds local constraints.

- The grammars are Semantic Unison's negotiated `org.firstpair.dng-xmp/0`, `/1` and
  `/2` profiles. Changing what any grammar admits changes those profiles: coordinate
  with Unison first and add synthetic controls for every widening.
- Refuse anything outside the grammar with a reason. Never guess, and never let a
  bound become unbounded.
- Tests use synthetic bytes built in memory only. Never commit photos, catalogs,
  personal paths or third-party binaries.
- Test in both switches: `make both`.
