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
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/subject-tests-%p%c.profraw" \
      swift test --package-path swift/subject --no-parallel \
        -Xswiftc -profile-generate \
        -Xswiftc -profile-coverage-mapping
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/runtime-tests-%p%c.profraw" \
      swift test --package-path swift/roam-runtime --no-parallel \
        -Xswiftc -profile-generate \
        -Xswiftc -profile-coverage-mapping
    RUNTIME_TEST_BIN="$(swift build --package-path swift/roam-runtime --show-bin-path)/roam-runtimePackageTests.xctest/Contents/MacOS/roam-runtimePackageTests" && \
      SUBJECT_TEST_BIN="$(swift build --package-path swift/subject --show-bin-path)/subject-swiftPackageTests.xctest/Contents/MacOS/subject-swiftPackageTests" && \
      xcrun llvm-profdata merge -sparse .coverage/swift-subject/*.profraw -o .coverage/swift-subject/subject.profdata && \
      xcrun llvm-cov report "$(pwd)/swift/subject/.build/debug/subject-swift" \
        -object "$RUNTIME_TEST_BIN" \
        -object "$SUBJECT_TEST_BIN" \
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
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/subject-tests-%p%c.profraw" \
      swift test --package-path swift/subject --no-parallel \
        -Xswiftc -profile-generate \
        -Xswiftc -profile-coverage-mapping
    LLVM_PROFILE_FILE="$(pwd)/.coverage/swift-subject/runtime-tests-%p%c.profraw" \
      swift test --package-path swift/roam-runtime --no-parallel \
        -Xswiftc -profile-generate \
        -Xswiftc -profile-coverage-mapping
    RUNTIME_TEST_BIN="$(swift build --package-path swift/roam-runtime --show-bin-path)/roam-runtimePackageTests.xctest/Contents/MacOS/roam-runtimePackageTests" && \
      SUBJECT_TEST_BIN="$(swift build --package-path swift/subject --show-bin-path)/subject-swiftPackageTests.xctest/Contents/MacOS/subject-swiftPackageTests" && \
      xcrun llvm-profdata merge -sparse .coverage/swift-subject/*.profraw -o .coverage/swift-subject/subject.profdata && \
      xcrun llvm-cov report "$(pwd)/swift/subject/.build/debug/subject-swift" \
        -object "$RUNTIME_TEST_BIN" \
        -object "$SUBJECT_TEST_BIN" \
        -instr-profile=.coverage/swift-subject/subject.profdata

swift-subject-cov-html:
    RUNTIME_TEST_BIN="$(swift build --package-path swift/roam-runtime --show-bin-path)/roam-runtimePackageTests.xctest/Contents/MacOS/roam-runtimePackageTests" && \
      SUBJECT_TEST_BIN="$(swift build --package-path swift/subject --show-bin-path)/subject-swiftPackageTests.xctest/Contents/MacOS/subject-swiftPackageTests" && \
      xcrun llvm-cov show "$(pwd)/swift/subject/.build/debug/subject-swift" \
        -object "$RUNTIME_TEST_BIN" \
        -object "$SUBJECT_TEST_BIN" \
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

fuzz-targets:
    @echo "Available fuzz targets:"
    @echo "  framing_peek         (fuzz/roam-shm-afl)"
    @echo "  shm_link_roundtrip   (fuzz/roam-shm-afl)"
    @echo "  protocol_decode      (fuzz/roam-afl)"
    @echo "  testbed_mem_session  (fuzz/roam-afl)"
    @echo ""
    @echo "Use: just fuzz-build [target|all]"
    @echo "Use: just fuzz-run [target|all] [seconds]"
    @echo "Use: just fuzz [target|all] [seconds]"

fuzz-build target="all":
    @case "{{target}}" in \
      all) \
        cargo afl build --manifest-path fuzz/roam-shm-afl/Cargo.toml --bin framing_peek; \
        cargo afl build --manifest-path fuzz/roam-shm-afl/Cargo.toml --bin shm_link_roundtrip; \
        cargo afl build --manifest-path fuzz/roam-afl/Cargo.toml --bin protocol_decode; \
        cargo afl build --manifest-path fuzz/roam-afl/Cargo.toml --bin testbed_mem_session; \
        ;; \
      framing_peek|shm_link_roundtrip) \
        cargo afl build --manifest-path fuzz/roam-shm-afl/Cargo.toml --bin "{{target}}"; \
        ;; \
      protocol_decode|testbed_mem_session) \
        cargo afl build --manifest-path fuzz/roam-afl/Cargo.toml --bin "{{target}}"; \
        ;; \
      *) \
        echo "Unknown target: {{target}}" >&2; \
        just fuzz-targets; \
        exit 1; \
        ;; \
    esac

fuzz-run target="all" seconds="60":
    just fuzz-build "{{target}}"
    @mkdir -p \
      fuzz/roam-shm-afl/out/framing_peek \
      fuzz/roam-shm-afl/out/shm_link_roundtrip \
      fuzz/roam-afl/out/protocol_decode \
      fuzz/roam-afl/out/testbed_mem_session
    @trap 'exit 130' INT TERM; \
    run_fuzz() { \
      cargo afl fuzz -V "$1" -i "$2" -o "$3" -- "$4"; \
      status=$?; \
      case "$status" in \
        0) ;; \
        130|143) exit "$status" ;; \
        *) exit "$status" ;; \
      esac; \
    }; \
    case "{{target}}" in \
      all) \
        run_fuzz "{{seconds}}" fuzz/roam-shm-afl/in/framing_peek fuzz/roam-shm-afl/out/framing_peek fuzz/roam-shm-afl/target/debug/framing_peek; \
        run_fuzz "{{seconds}}" fuzz/roam-shm-afl/in/shm_link_roundtrip fuzz/roam-shm-afl/out/shm_link_roundtrip fuzz/roam-shm-afl/target/debug/shm_link_roundtrip; \
        run_fuzz "{{seconds}}" fuzz/roam-afl/in/protocol_decode fuzz/roam-afl/out/protocol_decode fuzz/roam-afl/target/debug/protocol_decode; \
        run_fuzz "{{seconds}}" fuzz/roam-afl/in/testbed_mem_session fuzz/roam-afl/out/testbed_mem_session fuzz/roam-afl/target/debug/testbed_mem_session; \
        ;; \
      framing_peek) \
        run_fuzz "{{seconds}}" fuzz/roam-shm-afl/in/framing_peek fuzz/roam-shm-afl/out/framing_peek fuzz/roam-shm-afl/target/debug/framing_peek; \
        ;; \
      shm_link_roundtrip) \
        run_fuzz "{{seconds}}" fuzz/roam-shm-afl/in/shm_link_roundtrip fuzz/roam-shm-afl/out/shm_link_roundtrip fuzz/roam-shm-afl/target/debug/shm_link_roundtrip; \
        ;; \
      protocol_decode) \
        run_fuzz "{{seconds}}" fuzz/roam-afl/in/protocol_decode fuzz/roam-afl/out/protocol_decode fuzz/roam-afl/target/debug/protocol_decode; \
        ;; \
      testbed_mem_session) \
        run_fuzz "{{seconds}}" fuzz/roam-afl/in/testbed_mem_session fuzz/roam-afl/out/testbed_mem_session fuzz/roam-afl/target/debug/testbed_mem_session; \
        ;; \
      *) \
        echo "Unknown target: {{target}}" >&2; \
        just fuzz-targets; \
        exit 1; \
        ;; \
    esac

fuzz target="all" seconds="60":
    just fuzz-run "{{target}}" "{{seconds}}"
