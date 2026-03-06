import Foundation
import Testing

@testable import RoamRuntime

private actor GrantInbox {
    private var grants: [UInt32] = []

    func append(_ value: UInt32) {
        grants.append(value)
    }

    func snapshot() -> [UInt32] {
        grants
    }
}

private func withTimeout<T: Sendable>(
    milliseconds: UInt64,
    operation: @escaping @Sendable () async throws -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask {
            try await operation()
        }
        group.addTask {
            try await Task.sleep(nanoseconds: milliseconds * 1_000_000)
            throw POSIXError(.ETIMEDOUT)
        }
        let result = try await group.next()!
        group.cancelAll()
        return result
    }
}

struct ChannelFlowControlTests {
    @Test func senderWaitsForGrantCredit() async throws {
        let registry = ChannelRegistry()
        let credit = await registry.registerOutgoing(1, initialCredit: 1)
        let tx = Tx<Int32>(serialize: { encodeI32($0) })
        tx.bind(channelId: 1, taskTx: { _ in }, credit: credit)

        try await tx.send(1)

        let secondSend = Task {
            try await tx.send(2)
        }

        do {
            try await withTimeout(milliseconds: 50) {
                try await secondSend.value
            }
            Issue.record("second send should block until credit is granted")
        } catch let error as POSIXError {
            #expect(error.code == .ETIMEDOUT)
        }

        await registry.deliverCredit(channelId: 1, bytes: 1)
        try await withTimeout(milliseconds: 500) {
            try await secondSend.value
        }
    }

    @Test func receiverBatchesGrantCreditAtHalfWindow() async throws {
        let registry = ChannelRegistry()
        let inbox = GrantInbox()
        let receiver = await registry.register(7, initialCredit: 16) { additional in
            Task {
                await inbox.append(additional)
            }
        }

        for i in 0..<8 {
            receiver.deliver(encodeI32(Int32(i)))
        }

        for i in 0..<8 {
            let bytes = await receiver.recv()
            #expect(bytes != nil)
            var offset = 0
            let value = try decodeI32(from: Data(bytes!), offset: &offset)
            #expect(value == Int32(i))
        }

        let grants = await inbox.snapshot()
        #expect(grants == [8])
    }
}
