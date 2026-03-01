# cf. https://github.com/casey/just

list:
    just --list

rust *args:
    cargo build --package subject-rust
    SUBJECT_CMD="./target/debug/subject-rust" cargo nextest run -p spec-tests {{ quote(args) }}

cov *args:
    cargo llvm-cov nextest --summary-only {{ quote(args) }}

rust-ffi:
    cargo build --release -p roam-shm-ffi

ts-typecheck:
    pnpm check

ts-codegen:
    cargo xtask codegen --typescript

ts *args:
    just ts-typecheck
    just ts-codegen
    SUBJECT_CMD="sh typescript/subject/subject-ts.sh" cargo nextest run -p spec-tests {{ quote(args) }}

swift *args:
    just rust-ffi
    swift test --no-parallel -Xlinker -L$(pwd)/target/release
    swift build -c release --package-path swift/subject
    SUBJECT_CMD="sh swift/subject/subject-swift.sh" cargo nextest run -p spec-tests {{ quote(args) }}

all *args:
    just rust {{ quote(args) }}
    just ts {{ quote(args) }}
    just swift {{ quote(args) }}

wasm-build:
    wasm-pack build --target web rust/wasm-browser-tests --out-dir ../../typescript/tests/browser-wasm/pkg

ws-wasm *args:
    just wasm-build
    cd typescript/tests/playwright && pnpm exec playwright test ws-wasm.spec.ts {{ args }}

ws-ts *args:
    cd typescript/tests/playwright && pnpm exec playwright test ws-ts.spec.ts {{ args }}

fuzz-shm-build:
    cargo afl build --manifest-path fuzz/roam-shm-afl/Cargo.toml --bin framing_peek
    cargo afl build --manifest-path fuzz/roam-shm-afl/Cargo.toml --bin shm_link_roundtrip
