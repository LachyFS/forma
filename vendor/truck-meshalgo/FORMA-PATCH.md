# Forma patch: truck-meshalgo 0.4.0

This directory is the crates.io release `truck-meshalgo 0.4.0`
(Apache-2.0, <https://github.com/ricosjp/truck>, git sha
`487292c3686c4efd4b4c01acef9ff86136f6f630` per `.cargo_vcs_info.json`),
copied verbatim from the registry and used through `[patch.crates-io]` in the
workspace `Cargo.toml`. Exactly two lines differ from the published crate:

1. **`Cargo.toml`: `"vtk"` removed from the `default` feature list.**
   `truck-shapeops` and `truck-stepio` depend on `truck-meshalgo` with default
   features, which would pull `vtkio` and through it `lz4_flex` and
   `quick-xml` with open RustSec advisories (RUSTSEC-2026-0041,
   RUSTSEC-2026-0194, RUSTSEC-2026-0195) and an `lzma` C build. Forma never
   reads or writes VTK files. The `vtk` feature itself is still declared, so
   nothing that names it explicitly breaks.
2. **`src/lib.rs`: `#![cfg_attr(not(debug_assertions), deny(warnings))]`
   deleted.** Path dependencies do not get `--cap-lints`, so any new rustc
   warning in a future toolchain would otherwise fail Forma's release builds.

The registry's `.cargo-ok` marker and the crate's own `Cargo.lock` (ignored by
Cargo for dependencies) were not copied. Nothing else was changed: no source
edits, no new dependencies.

To verify:

```sh
diff -r ~/.cargo/registry/src/index.crates.io-*/truck-meshalgo-0.4.0 vendor/truck-meshalgo
```

Expected output: the two hunks above, `Only in` lines for `.cargo-ok` and
`Cargo.lock`, and `Only in vendor/truck-meshalgo: FORMA-PATCH.md`.

Drop this patch once a truck-meshalgo release makes `vtk` opt-in (upstream
master already pins newer versions; re-run the RustSec check when upgrading).
