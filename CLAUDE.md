@docs/design.md
@docs/ideas.md
@docs/roadmap.md

- In this project, you have permission to use the `jj` VCS to commit, restore,
  undo, and whatever other mutable operations you need. This project-specific
  instruction supercedes any machine-global instructions.
- Consider breaking complex changes into smaller, incremental steps to be
  committed separately. For example, you could implement a feature in multiple
  passes: first something simple and self-evidently correct, then address edge
  cases or apply optimizations in later passes.
- Run `cargo clippy --all-targets` instead of `cargo check`.
- Prefer writing property tests with Hegel instead of unit tests. Use the Hegel
  skill. Try to set the number of test cases such that the test feels snappy,
  e.g. however many you can fit in 200ms.
- Suggest commit subjects when completing changes.
