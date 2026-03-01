#if os(macOS)
import Foundation
import Testing

@testable import RoamRuntime

private func loadWireV7Fixture(_ name: String) throws -> [UInt8] {
    let testFile = URL(fileURLWithPath: #filePath)
    let projectRoot =
        testFile
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
    let path = projectRoot.appendingPathComponent("test-fixtures/golden-vectors/wire-v7/\(name).bin")
    return Array(try Data(contentsOf: path))
}

private func samePostcardError(_ lhs: PostcardError, _ rhs: PostcardError) -> Bool {
    switch (lhs, rhs) {
    case (.truncated, .truncated), (.invalidUtf8, .invalidUtf8), (.unknownVariant, .unknownVariant),
         (.overflow, .overflow):
        return true
    default:
        return false
    }
}

private func expectWireError(_ expected: WireV7Error, _ body: () throws -> Void) {
    do {
        try body()
        Issue.record("expected WireV7Error \(expected)")
    } catch let actual as WireV7Error {
        #expect(actual == expected)
    } catch {
        Issue.record("expected WireV7Error \(expected), got \(error)")
    }
}

private func expectPostcardError(_ expected: PostcardError, _ body: () throws -> Void) {
    do {
        try body()
        Issue.record("expected PostcardError \(expected)")
    } catch let actual as PostcardError {
        #expect(samePostcardError(actual, expected))
    } catch {
        Issue.record("expected PostcardError \(expected), got \(error)")
    }
}

struct WireV7PostcardNegativeTests {
    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsTrailingBytes() throws {
        var bytes = try loadWireV7Fixture("message_hello")
        bytes.append(0xFF)
        expectWireError(.trailingBytes) {
            _ = try MessageV7.decode(from: Data(bytes))
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsUnknownPayloadVariant() {
        expectWireError(.unknownVariant(9)) {
            _ = try MessageV7.decode(from: Data([0, 9]))
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsTruncatedStringField() {
        let truncatedProtocolError: [UInt8] = [0, 2, 3, 0x41, 0x42]
        expectWireError(.truncated) {
            _ = try MessageV7.decode(from: Data(truncatedProtocolError))
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsInvalidParityVariant() {
        let bytes: [UInt8] = [0, 0, 7, 3, 1, 0]
        expectWireError(.unknownVariant(3)) {
            _ = try MessageV7.decode(from: Data(bytes))
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsOverflowingU32Field() {
        var bytes: [UInt8] = []
        bytes += encodeVarint(0) // connection_id
        bytes += encodeVarint(0) // payload: hello
        bytes += encodeVarint(7) // hello.version
        bytes += encodeVarint(0) // parity: odd
        bytes += [0x80, 0x80, 0x80, 0x80, 0x10] // max_concurrent_requests > u32
        bytes += encodeVarint(0) // metadata: empty vec

        expectWireError(.overflow) {
            _ = try MessageV7.decode(from: Data(bytes))
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func wireDecodeRejectsInvalidUtf8InStringField() {
        let bytes: [UInt8] = [0, 2, 2, 0xC3, 0x28]
        expectWireError(.invalidUtf8) {
            _ = try MessageV7.decode(from: Data(bytes))
        }
    }

    // r[verify rpc.channel.payload-encoding]
    @Test func postcardDecodeBytesRejectsTruncatedLength() {
        var offset = 0
        expectPostcardError(.truncated) {
            _ = try decodeBytes(from: Data([4, 1, 2]), offset: &offset)
        }
    }

    // r[verify rpc.channel.payload-encoding]
    @Test func postcardDecodeStringRejectsInvalidUtf8() {
        var offset = 0
        expectPostcardError(.invalidUtf8) {
            _ = try decodeString(from: Data([2, 0xC3, 0x28]), offset: &offset)
        }
    }

    // r[verify rpc.channel.payload-encoding]
    @Test func postcardDecodeU32RejectsOverflow() {
        var offset = 0
        expectPostcardError(.overflow) {
            _ = try decodeU32(from: Data([0x80, 0x80, 0x80, 0x80, 0x10]), offset: &offset)
        }
    }

    // r[verify rpc.fallible.roam-error]
    @Test func decodeRpcResultRejectsInvalidOuterDiscriminant() {
        var offset = 0
        do {
            try decodeRpcResult(from: Data([2]), offset: &offset)
            Issue.record("expected invalid outer Result discriminant error")
        } catch let error as RoamError {
            guard case .decodeError(let message) = error else {
                Issue.record("expected RoamError.decodeError, got \(error)")
                return
            }
            #expect(message.contains("invalid outer Result discriminant"))
        } catch {
            Issue.record("expected RoamError.decodeError, got \(error)")
        }
    }

    // r[verify rpc.fallible.roam-error]
    @Test func decodeRpcResultRejectsUnknownRoamErrorDiscriminant() {
        var offset = 0
        do {
            try decodeRpcResult(from: Data([1, 9]), offset: &offset)
            Issue.record("expected invalid RoamError discriminant error")
        } catch let error as RoamError {
            guard case .decodeError(let message) = error else {
                Issue.record("expected RoamError.decodeError, got \(error)")
                return
            }
            #expect(message.contains("invalid RoamError discriminant"))
        } catch {
            Issue.record("expected RoamError.decodeError, got \(error)")
        }
    }
}
#endif
