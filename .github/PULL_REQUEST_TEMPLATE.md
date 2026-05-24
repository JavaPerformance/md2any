## Summary

<!-- One or two sentences describing what changed and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / cleanup
- [ ] Documentation only
- [ ] Tooling / CI

## Output formats affected

- [ ] PPTX
- [ ] ODP
- [ ] PDF
- [ ] DOCX
- [ ] ODT
- [ ] None / cross-cutting

## Checklist

- [ ] `cargo build --release` passes
- [ ] `cargo test --release` passes
- [ ] `cargo clippy --all-targets -- -D warnings` is clean (or noted otherwise)
- [ ] `cargo fmt` has been run
- [ ] `CHANGELOG.md` updated under `[Unreleased]` for user-visible changes
- [ ] Snapshot tests updated (`UPDATE_SNAPSHOTS=1 cargo test`) if parser /
      paginator output changed intentionally

## Notes for reviewers

<!-- Anything reviewers should know that isn't obvious from the diff: edge
cases you considered, alternative approaches you rejected, follow-up work
that's intentionally out of scope. -->
