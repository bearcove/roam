# telex AFL Fuzzing

`cargo-afl` harnesses for Rust Telex protocol/state-machine behavior.

## Targets

- `protocol_decode`
  - Feeds arbitrary bytes into Telex postcard decode for `telex_types::Message`.
  - Re-encodes successfully decoded messages.
- `testbed_mem_session`
  - Runs generated `spec-proto` Testbed RPC traffic over in-memory initiator/acceptor+driver.
  - Exercises unary + streaming calls (`sum`, `generate`, `transform`) with fuzz-derived inputs.

## Build

```bash
cargo afl build --manifest-path fuzz/telex-afl/Cargo.toml --bin protocol_decode
cargo afl build --manifest-path fuzz/telex-afl/Cargo.toml --bin testbed_mem_session
```

## Run

```bash
cargo afl fuzz \
  -i fuzz/telex-afl/in/protocol_decode \
  -o fuzz/telex-afl/out/protocol_decode \
  -- fuzz/telex-afl/target/debug/protocol_decode

cargo afl fuzz \
  -i fuzz/telex-afl/in/testbed_mem_session \
  -o fuzz/telex-afl/out/testbed_mem_session \
  -- fuzz/telex-afl/target/debug/testbed_mem_session
```

For smoke runs:

```bash
timeout 60 cargo afl fuzz ...
```
