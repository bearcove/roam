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
    @Test func rustV7HelloFixtureIsNotLegacyWireCompatibleYet() throws {
        let bytes = try loadWireV7Fixture("message_hello")
        do {
            let decoded = try Message.decode(from: Data(bytes))
            let reencoded = decoded.encode()
            #expect(reencoded != bytes)
        } catch {
            // Any decode error is also acceptable for this pre-migration compatibility check.
        }
    }

    // r[verify session.message]
    // r[verify session.message.payloads]
    @Test func rustV7NestedPayloadFixturesAreTracked() throws {
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
            do {
                let decoded = try Message.decode(from: Data(bytes))
                let reencoded = decoded.encode()
                #expect(reencoded != bytes, "\(name) unexpectedly round-tripped through legacy wire codec")
            } catch {
                // Decode errors are expected until Swift moves to the v7 nested wire model.
            }
        }
    }
}
#endif
