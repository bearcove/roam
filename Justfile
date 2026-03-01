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
    SPEC_TRANSPORT=shm SUBJECT_CMD="sh swift/subject/subject-swift.sh" cargo nextest run -p spec-tests {{ quote(args) }}

swift-subject-cov *args:
    just rust-ffi
    rm -rf .coverage/swift-subject
    mkdir -p .coverage/swift-subject
    swift build -c debug --package-path swift/subject --product subject-swift \
      -Xswiftc -profile-generate \
      -Xswiftc -profile-coverage-mapping
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/tcp-%p%c.profraw" \
      SUBJECT_CMD="$(pwd)/swift/subject/.build/debug/subject-swift" \
      cargo nextest run -P coverage --test-threads=1 -p spec-tests {{ quote(args) }}
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/shm-%p%c.profraw" \
      SPEC_TRANSPORT=shm \
      SUBJECT_CMD="$(pwd)/swift/subject/.build/debug/subject-swift" \
      cargo nextest run -P coverage --test-threads=1 -p spec-tests {{ quote(args) }}
    xcrun llvm-profdata merge -sparse .coverage/swift-subject/*.profraw -o .coverage/swift-subject/subject.profdata
    xcrun llvm-cov report "$(pwd)/swift/subject/.build/debug/subject-swift" \
      -instr-profile=.coverage/swift-subject/subject.profdata

swift-subject-cov-tcp *args:
    just rust-ffi
    rm -rf .coverage/swift-subject
    mkdir -p .coverage/swift-subject
    swift build -c debug --package-path swift/subject --product subject-swift \
      -Xswiftc -profile-generate \
      -Xswiftc -profile-coverage-mapping
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/tcp-%p%c.profraw" \
      SUBJECT_CMD="$(pwd)/swift/subject/.build/debug/subject-swift" \
      cargo nextest run -P coverage --test-threads=1 -p spec-tests {{ quote(args) }}
    xcrun llvm-profdata merge -sparse .coverage/swift-subject/*.profraw -o .coverage/swift-subject/subject.profdata
    xcrun llvm-cov report "$(pwd)/swift/subject/.build/debug/subject-swift" \
      -instr-profile=.coverage/swift-subject/subject.profdata

swift-subject-cov-html:
    xcrun llvm-cov show "$(pwd)/swift/subject/.build/debug/subject-swift" \
      -instr-profile=.coverage/swift-subject/subject.profdata \
      -format=html -output-dir .coverage/swift-subject/html

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
