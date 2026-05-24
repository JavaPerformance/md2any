# Contributing to md2any

Thanks for your interest in md2any! This document covers how to report issues,
propose changes, and get a development environment running locally.

## Reporting issues

Before opening an issue, please check the
[existing issues](https://github.com/javaperformance/md2any/issues) to see
whether the bug or feature is already tracked.

A good bug report includes:

1. The exact `md2any --version` output.
2. The platform and architecture (`uname -a` on Linux/macOS, OS version on
   Windows).
3. A **minimal** markdown sample that reproduces the issue — five lines is
   better than five hundred.
4. The exact command line you ran.
5. The expected output vs what actually happened.

For renderer bugs (PPTX/ODP/DOCX/ODT showing wrong layout), please attach a
screenshot or the failing output file if you can — visual bugs are hard to
describe in text.

## Development setup

You need a stable Rust toolchain. Rustup is the easiest way to get one:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then:

```bash
git clone https://github.com/javaperformance/md2any
cd md2any
cargo build --release
cargo test
```

The repository works the same on Linux, macOS, and Windows — there are no
platform-specific dependencies.

### Useful one-liners

```bash
cargo build --release                    # build the binary
cargo test --release                     # run all tests
cargo test --release -- --nocapture      # tests with stdout
cargo test --release -- --ignored        # run opt-in integration tests too
cargo clippy --all-targets -- -D warnings   # lint
cargo fmt                                # format
cargo doc --no-deps --open               # build + open API docs

./target/release/md2any examples/demo.md
./target/release/md2any examples/demo.md -o demo.pdf
./target/release/md2any examples/demo.md --serve   # localhost preview
```

The `--ignored` set currently contains the remote-image cache stress
harness (`tests/integration/cache_stress.sh`). It needs Python 3 and curl
on `$PATH` and a free local port; the test reports `exit 77` (skip) if
prerequisites are missing, so it's safe to run anywhere. See
`tests/integration/README.md` for details and how to add new scenarios.

### Cross-compilation

`scripts/build-all.sh` builds release binaries for every supported target.
Set up the targets once:

```bash
rustup target add x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-gnu
rustup target add x86_64-pc-windows-msvc
rustup target add x86_64-apple-darwin aarch64-apple-darwin
```

Then:

```bash
./scripts/build-all.sh                  # all targets
./scripts/build-all.sh --linux          # just Linux variants
```

On GitHub, the [`release.yml`](.github/workflows/release.yml) workflow
publishes binaries for every push of a `v*` tag.

## Code style

- `cargo fmt` is enforced — CI fails on diffs.
- `cargo clippy` is treated as warnings → errors. Don't silence lints
  without an explanatory comment.
- Public items get rustdoc comments (`///`) and types/modules get a top-level
  `//!` description. Internal helpers don't need docs but should have names
  that tell you why they exist.
- Comments explain **why**, not **what**. The code already says what it does.

## Tests

- Snapshot tests live in `tests/snapshots/*.md` with golden files
  `tests/snapshots/*.snap`. They cover the parser → paginator pipeline.
- To accept an intentional change after touching the parser or paginator:
  `UPDATE_SNAPSHOTS=1 cargo test`.
- Renderer tests are not yet exhaustive — verifying a real PPTX / DOCX in
  PowerPoint or LibreOffice is currently a manual step. Patches that add
  golden output bytes are welcome.

## Adding a new output format

The IR (`src/ir.rs`) is renderer-agnostic by design. To add a sixth output
format:

1. Add a new variant to `enum Format` in `src/main.rs`.
2. Wire `from_extension` / `from_name` / `ext` / `name`.
3. Write `pub fn write(slides, theme, ...) -> Result<Vec<u8>>` in a new
   `src/<format>.rs`.
4. Re-export the module in `src/lib.rs`.
5. Hook the writer into the match in `main.rs`.

The existing five writers (`pptx.rs`, `odp.rs`, `pdf.rs`, `docx.rs`, `odt.rs`)
are independent of each other — copying the closest one as a template and
modifying is the recommended approach.

## Pull requests

- Keep PRs focused. One bug fix or one feature per PR.
- Update `CHANGELOG.md` under `[Unreleased]` with what changed and why.
- New features need at least one snapshot test or an example markdown file.
- The CI workflow runs `cargo build`, `cargo test`, and a smoke test on
  every push.

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
By participating, you agree to abide by its terms.

## License

By contributing, you agree that your contributions will be dual-licensed
under the MIT and Apache-2.0 licenses, as described in the
[README](README.md#license).
