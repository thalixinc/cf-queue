# cfq

## Verifying your work

Run these before any "done"; the full suite green is the definition of done for every ticket (crew rule 5).

```sh
cargo build
cargo test
cargo clippy
```

Evidence (the command output) goes to `intent/<epic>-<slug>/tickets/<n>-<slug>/evidence/`.
