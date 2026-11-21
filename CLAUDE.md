@docs/design.md
@docs/ideas.md
@docs/roadmap.md

- Consider breaking complex changes into smaller, incremental steps to be
  committed separately. For example, you could implement a feature in multiple
  passes: first something simple and self-evidently correct, then address edge
  cases or apply optimizations in later passes.
- Run `cargo clippy --all-targets` instead of `cargo check`.
- Prefer writing property tests with Hegel instead of unit tests.
- Suggest commit subjects when completing changes.
