# telex-shm AFL Fuzzing

This directory contains `cargo-afl` harnesses for `telex-shm`.

## Targets

- `framing_peek`: fuzzes SHM frame parsing (`framing::peek_frame`)
- `shm_link_roundtrip`: fuzzes `ShmLink` send/recv roundtrip with bounded payloads

## Prerequisites

1. Install AFL tooling:
   - `cargo install cargo-afl`
2. Linux host recommended for best AFL support.

## Build Harnesses

```bash
cargo afl build --manifest-path fuzz/telex-shm-afl/Cargo.toml --bin framing_peek
cargo afl build --manifest-path fuzz/telex-shm-afl/Cargo.toml --bin shm_link_roundtrip
```

## Run Fuzzers

```bash
cargo afl fuzz \
  -i fuzz/telex-shm-afl/in/framing_peek \
  -o fuzz/telex-shm-afl/out/framing_peek \
  -- fuzz/telex-shm-afl/target/debug/framing_peek

cargo afl fuzz \
  -i fuzz/telex-shm-afl/in/shm_link_roundtrip \
  -o fuzz/telex-shm-afl/out/shm_link_roundtrip \
  -- fuzz/telex-shm-afl/target/debug/shm_link_roundtrip
```

To limit run time in CI/manual smoke runs:

```bash
timeout 60 cargo afl fuzz ...
```
