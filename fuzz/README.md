# Fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for margo's
parsers. Needs a nightly toolchain and `cargo install cargo-fuzz`.

| Target | What it fuzzes |
| --- | --- |
| `parse_config_str` | `margo_config::parse_config_str`, the compositor `.conf` parser |
| `validate_config` | `margo_config::validator::validate_config`, the `mctl check-config` validator |

Both targets blank `source = …` / `include = …` lines first (they read other
files from disk, so a run would depend on the machine); every other line is
fuzzed. CI compiles the targets and tests that filter on stable (`ci.yml`) and
fuzzes each target for 10 minutes every week on nightly (`soak.yml`).

Run (from the repo root), seeding the corpus with the shipped example config
cut into small pieces — whole ~95 KiB configs make every run slow (the same
for `validate_config`):

```sh
mkdir -p fuzz/corpus/parse_config_str
grep -v '^[[:space:]]*#' margo/src/config.example.conf | grep -v '^[[:space:]]*$' |
  split -l 16 - fuzz/corpus/parse_config_str/example-
cargo +nightly fuzz run parse_config_str -- -max_total_time=120 -max_len=4096
```

A crash is saved under `fuzz/artifacts/<target>/`; replay it with
`cargo +nightly fuzz run <target> <file>` and add the input as a
regression test in `margo-config` before fixing it.
