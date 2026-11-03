# Fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for margo's
parsers. Needs a nightly toolchain and `cargo install cargo-fuzz`.

| Target | What it fuzzes |
| --- | --- |
| `parse_config_str` | `margo_config::parse_config_str`, the compositor `.conf` parser |

Run (from the repo root), seeding the corpus with the shipped example config:

```sh
mkdir -p fuzz/corpus/parse_config_str
cp margo/src/config.example.conf fuzz/corpus/parse_config_str/
cargo +nightly fuzz run parse_config_str -- -max_total_time=120
```

A crash is saved under `fuzz/artifacts/parse_config_str/`; replay it with
`cargo +nightly fuzz run parse_config_str <file>` and add the input as a
regression test in `margo-config` before fixing it.
