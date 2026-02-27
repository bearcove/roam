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

struct WireV7CrossLanguageTests {
    // r[verify session.message]
    // r[verify session.message.payloads]
    // r[verify session.connection-settings.hello]
    @Test func rustV7HelloFixtureRoundTripsInV7Codec() throws {
        let bytes = try loadWireV7Fixture("message_hello")
        let decoded = try MessageV7.decode(from: Data(bytes))
        #expect(decoded.connectionId == 0)
        guard case .hello(let hello) = decoded.payload else {
            Issue.record("expected hello payload")
            return
        }
        #expect(hello.version == 7)
        #expect(hello.connectionSettings.parity == .odd)
        #expect(hello.connectionSettings.maxConcurrentRequests == 64)
        #expect(decoded.encode() == bytes)
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func rustV7NestedPayloadFixturesRoundTrip() throws {
        let names = [
            "message_hello",
            "message_hello_yourself",
            "message_protocol_error",
            "message_connection_open",
            "message_connection_accept",
            "message_connection_reject",
            "message_connection_close",
            "message_request_call",
            "message_request_response",
            "message_request_cancel",
            "message_channel_item",
            "message_channel_close",
            "message_channel_reset",
            "message_channel_grant_credit",
        ]

        for name in names {
            let bytes = try loadWireV7Fixture(name)
            #expect(!bytes.isEmpty)
            let decoded = try MessageV7.decode(from: Data(bytes))
            #expect(decoded.encode() == bytes, "\(name) failed byte-for-byte round trip")
        }
    }
}
#endif
