# Integration tests

End-to-end checks that spin up local services and drive the compiled
binary. Excluded from the default `cargo test` run because they need
extra prerequisites (Python 3, curl, a network port).

## What's here

| File | What it covers |
|------|----------------|
| `cache_server.py` | Tiny HTTP test server with controllable misbehaviour: valid PNG, flaky 503-then-200, empty body, garbage body, 30 MB payload, request counter, retry reset |
| `cache_stress.sh` | 9-scenario stress harness driving md2any against the server. Asserts cold fetch, warm cache, URL normalisation, `--no-remote-image-cache`, retry-on-503, 20 MB cap, garbage rejection, empty-body rejection, concurrent-render `.tmp` safety |

## How to run

The Rust wrapper:

```sh
cargo build --release
cargo test --test cache_stress -- --ignored --nocapture
```

Or the shell harness directly (useful when iterating on `src/image.rs`):

```sh
cargo build --release
tests/integration/cache_stress.sh
```

The harness exits 0 on full pass, 77 (skip) when prerequisites are
missing, anything else means a real assertion failed.

## Adding a new scenario

1. Add the synthetic route to `cache_server.py` (keep responses minimal).
2. Add a `note "N. description"` block to `cache_stress.sh` with `pass` /
   `fail` calls. Increment the assertion total in the wrapper's header
   comment if you change the count materially.
3. Re-run; the Rust wrapper will pick it up automatically.

## Why Python + bash?

These tests need a port, request-state tracking, and shell orchestration
that's awkward in pure Rust without adding a HTTP server crate as a
dev-dep. The Python server is ~120 lines, the bash driver is ~150 lines,
and both run on every common dev machine without extra installs.
