import Foundation
import Testing

@testable import RoamRuntime

/// Load a golden vector from the test-fixtures directory.
func loadGoldenVector(_ path: String) throws -> [UInt8] {
    let testFile = URL(fileURLWithPath: #filePath)
    let projectRoot =
        testFile
        .deletingLastPathComponent()  // RoamRuntimeTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // roam-runtime
        .deletingLastPathComponent()  // swift
        .deletingLastPathComponent()  // roam (project root)
    let vectorPath = projectRoot.appendingPathComponent("test-fixtures/golden-vectors/\(path)")
    let data = try Data(contentsOf: vectorPath)
    return Array(data)
}

/// Assert encoding matches golden vector
func assertEncoding(_ encoded: [UInt8], _ vectorPath: String) throws {
    let expected = try loadGoldenVector(vectorPath)
    if encoded != expected {
        Issue.record("Encoding mismatch for \(vectorPath): got \(encoded), expected \(expected)")
    }
}

// MARK: - Wire Protocol Tests

struct WireEncodingTests {
    @Test func testMessageV7HelloDecodeAndRoundtrip() throws {
        let bytes = try loadGoldenVector("wire-v7/message_hello.bin")
        let msg = try MessageV7.decode(from: Data(bytes))
        guard case .hello(let hello) = msg.payload else {
            Issue.record("Expected HelloV7 payload")
            return
        }
        #expect(msg.connectionId == 0)
        #expect(hello.version == 7)
        #expect(hello.connectionSettings.maxConcurrentRequests == 64)
        #expect(msg.encode() == bytes)
    }

    @Test func testMessageV7RequestCallDecodeAndRoundtrip() throws {
        let bytes = try loadGoldenVector("wire-v7/message_request_call.bin")
        let msg = try MessageV7.decode(from: Data(bytes))
        guard case .requestMessage(let request) = msg.payload,
            case .call(let call) = request.body
        else {
            Issue.record("Expected RequestCall payload")
            return
        }
        #expect(msg.connectionId == 2)
        #expect(request.id == 11)
        #expect(call.channels == [3, 5])
        #expect(!call.args.bytes.isEmpty)
        #expect(msg.encode() == bytes)
    }
}

// MARK: - Primitive Encoding Tests

struct PrimitiveEncodingTests {

    // MARK: - Bool

    @Test func testBoolEncoding() throws {
        try assertEncoding(encodeBool(false), "primitives/bool_false.bin")
        try assertEncoding(encodeBool(true), "primitives/bool_true.bin")
    }

    // MARK: - u8 / i8

    @Test func testU8Encoding() throws {
        try assertEncoding(encodeU8(0), "primitives/u8_0.bin")
        try assertEncoding(encodeU8(127), "primitives/u8_127.bin")
        try assertEncoding(encodeU8(255), "primitives/u8_255.bin")
    }

    @Test func testI8Encoding() throws {
        try assertEncoding(encodeI8(0), "primitives/i8_0.bin")
        try assertEncoding(encodeI8(-1), "primitives/i8_neg1.bin")
        try assertEncoding(encodeI8(127), "primitives/i8_127.bin")
        try assertEncoding(encodeI8(-128), "primitives/i8_neg128.bin")
    }

    // MARK: - u16 / i16

    @Test func testU16Encoding() throws {
        try assertEncoding(encodeU16(0), "primitives/u16_0.bin")
        try assertEncoding(encodeU16(127), "primitives/u16_127.bin")
        try assertEncoding(encodeU16(128), "primitives/u16_128.bin")
        try assertEncoding(encodeU16(255), "primitives/u16_255.bin")
        try assertEncoding(encodeU16(256), "primitives/u16_256.bin")
        try assertEncoding(encodeU16(UInt16.max), "primitives/u16_max.bin")
    }

    @Test func testI16Encoding() throws {
        try assertEncoding(encodeI16(0), "primitives/i16_0.bin")
        try assertEncoding(encodeI16(1), "primitives/i16_1.bin")
        try assertEncoding(encodeI16(-1), "primitives/i16_neg1.bin")
        try assertEncoding(encodeI16(127), "primitives/i16_127.bin")
        try assertEncoding(encodeI16(128), "primitives/i16_128.bin")
        try assertEncoding(encodeI16(Int16.max), "primitives/i16_max.bin")
        try assertEncoding(encodeI16(Int16.min), "primitives/i16_min.bin")
    }

    // MARK: - u32 / i32

    @Test func testU32Encoding() throws {
        try assertEncoding(encodeU32(0), "primitives/u32_0.bin")
        try assertEncoding(encodeU32(1), "primitives/u32_1.bin")
        try assertEncoding(encodeU32(127), "primitives/u32_127.bin")
        try assertEncoding(encodeU32(128), "primitives/u32_128.bin")
        try assertEncoding(encodeU32(255), "primitives/u32_255.bin")
        try assertEncoding(encodeU32(256), "primitives/u32_256.bin")
        try assertEncoding(encodeU32(UInt32.max), "primitives/u32_max.bin")
    }

    @Test func testI32Encoding() throws {
        try assertEncoding(encodeI32(0), "primitives/i32_0.bin")
        try assertEncoding(encodeI32(1), "primitives/i32_1.bin")
        try assertEncoding(encodeI32(-1), "primitives/i32_neg1.bin")
        try assertEncoding(encodeI32(127), "primitives/i32_127.bin")
        try assertEncoding(encodeI32(128), "primitives/i32_128.bin")
        try assertEncoding(encodeI32(-128), "primitives/i32_neg128.bin")
        try assertEncoding(encodeI32(Int32.max), "primitives/i32_max.bin")
        try assertEncoding(encodeI32(Int32.min), "primitives/i32_min.bin")
    }

    // MARK: - u64 / i64

    @Test func testU64Encoding() throws {
        try assertEncoding(encodeU64(0), "primitives/u64_0.bin")
        try assertEncoding(encodeU64(1), "primitives/u64_1.bin")
        try assertEncoding(encodeU64(127), "primitives/u64_127.bin")
        try assertEncoding(encodeU64(128), "primitives/u64_128.bin")
        try assertEncoding(encodeU64(UInt64.max), "primitives/u64_max.bin")
    }

    @Test func testI64Encoding() throws {
        try assertEncoding(encodeI64(0), "primitives/i64_0.bin")
        try assertEncoding(encodeI64(1), "primitives/i64_1.bin")
        try assertEncoding(encodeI64(-1), "primitives/i64_neg1.bin")
        try assertEncoding(encodeI64(15), "primitives/i64_15.bin")
        try assertEncoding(encodeI64(42), "primitives/i64_42.bin")
        try assertEncoding(encodeI64(Int64.max), "primitives/i64_max.bin")
        try assertEncoding(encodeI64(Int64.min), "primitives/i64_min.bin")
    }

    // MARK: - f32 / f64

    @Test func testF32Encoding() throws {
        try assertEncoding(encodeF32(0.0), "primitives/f32_0.bin")
        try assertEncoding(encodeF32(1.0), "primitives/f32_1.bin")
        try assertEncoding(encodeF32(-1.0), "primitives/f32_neg1.bin")
        try assertEncoding(encodeF32(1.5), "primitives/f32_1_5.bin")
        try assertEncoding(encodeF32(0.25), "primitives/f32_0_25.bin")
    }

    @Test func testF64Encoding() throws {
        try assertEncoding(encodeF64(0.0), "primitives/f64_0.bin")
        try assertEncoding(encodeF64(1.0), "primitives/f64_1.bin")
        try assertEncoding(encodeF64(-1.0), "primitives/f64_neg1.bin")
        try assertEncoding(encodeF64(1.5), "primitives/f64_1_5.bin")
        try assertEncoding(encodeF64(0.25), "primitives/f64_0_25.bin")
    }

    // MARK: - String

    @Test func testStringEncoding() throws {
        try assertEncoding(encodeString(""), "primitives/string_empty.bin")
        try assertEncoding(encodeString("hello world"), "primitives/string_hello.bin")
        try assertEncoding(encodeString("héllo 世界 🦀"), "primitives/string_unicode.bin")
    }

    // MARK: - Bytes

    @Test func testBytesEncoding() throws {
        try assertEncoding(encodeBytes([]), "primitives/bytes_empty.bin")
        try assertEncoding(encodeBytes([0xDE, 0xAD, 0xBE, 0xEF]), "primitives/bytes_deadbeef.bin")
    }

    // MARK: - Option

    @Test func testOptionEncoding() throws {
        try assertEncoding(
            encodeOption(nil as UInt32?, encoder: encodeU32), "primitives/option_none_u32.bin")
        try assertEncoding(
            encodeOption(42 as UInt32?, encoder: encodeU32), "primitives/option_some_u32_42.bin")
        try assertEncoding(
            encodeOption(nil as String?, encoder: encodeString), "primitives/option_none_string.bin"
        )
        try assertEncoding(
            encodeOption("hello" as String?, encoder: encodeString),
            "primitives/option_some_string.bin")
    }

    // MARK: - Vec

    @Test func testVecEncoding() throws {
        try assertEncoding(
            encodeVec([] as [UInt32], encoder: encodeU32), "primitives/vec_empty_u32.bin")
        try assertEncoding(
            encodeVec([1, 2, 3] as [UInt32], encoder: encodeU32), "primitives/vec_u32_1_2_3.bin")
        try assertEncoding(
            encodeVec([-1, 0, 1] as [Int32], encoder: encodeI32), "primitives/vec_i32_neg1_0_1.bin")
        try assertEncoding(
            encodeVec(["a", "b"], encoder: encodeString), "primitives/vec_string.bin")
    }
}

// MARK: - Varint Tests

struct VarintEncodingTests {

    @Test func testVarintEncoding() throws {
        try assertEncoding(encodeVarint(0), "varint/u64_0.bin")
        try assertEncoding(encodeVarint(1), "varint/u64_1.bin")
        try assertEncoding(encodeVarint(127), "varint/u64_127.bin")
        try assertEncoding(encodeVarint(128), "varint/u64_128.bin")
        try assertEncoding(encodeVarint(255), "varint/u64_255.bin")
        try assertEncoding(encodeVarint(256), "varint/u64_256.bin")
        try assertEncoding(encodeVarint(16383), "varint/u64_16383.bin")
        try assertEncoding(encodeVarint(16384), "varint/u64_16384.bin")
        try assertEncoding(encodeVarint(65535), "varint/u64_65535.bin")
        try assertEncoding(encodeVarint(65536), "varint/u64_65536.bin")
        try assertEncoding(encodeVarint(1_048_576), "varint/u64_1048576.bin")
    }
}
