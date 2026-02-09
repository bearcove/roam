import Foundation
import RoamRuntime

struct SpawnArgs {
    let hubPath: String
    let peerId: UInt8
    let doorbellFd: Int32
}

private func fail(_ message: String) -> Never {
    fputs("shm-guest-client: \(message)\n", stderr)
    exit(1)
}

private func parseArgs(_ args: [String]) -> SpawnArgs {
    var hubPath: String?
    var peerId: UInt8?
    var doorbellFd: Int32?

    for arg in args {
        if let value = arg.split(separator: "=", maxSplits: 1).last, arg.hasPrefix("--hub-path=") {
            hubPath = String(value)
        } else if let value = arg.split(separator: "=", maxSplits: 1).last, arg.hasPrefix("--peer-id=") {
            peerId = UInt8(value)
        } else if let value = arg.split(separator: "=", maxSplits: 1).last, arg.hasPrefix("--doorbell-fd=") {
            guard let fd = Int32(value) else {
                fail("invalid --doorbell-fd value")
            }
            doorbellFd = fd
        }
    }

    guard let hubPath else {
        fail("missing --hub-path")
    }
    guard let peerId else {
        fail("missing --peer-id")
    }
    guard let doorbellFd else {
        fail("missing --doorbell-fd")
    }

    return SpawnArgs(hubPath: hubPath, peerId: peerId, doorbellFd: doorbellFd)
}

let args = parseArgs(Array(CommandLine.arguments.dropFirst()))

let ticket = ShmBootstrapTicket(peerId: args.peerId, hubPath: args.hubPath, doorbellFd: args.doorbellFd)
let guest: ShmGuestRuntime

do {
    guest = try ShmGuestRuntime.attach(ticket: ticket)
} catch {
    fail("attach failed: \(error)")
}

let inlinePayload = Array("swift-inline".utf8)
let slotPayload = (0..<2048).map { UInt8(truncatingIfNeeded: $0) }

let inlineFrame = ShmGuestFrame(msgType: 4, id: 1, methodId: 0, payload: inlinePayload)
let slotFrame = ShmGuestFrame(msgType: 4, id: 2, methodId: 0, payload: slotPayload)

do {
    try guest.send(frame: inlineFrame)
    try guest.send(frame: slotFrame)
} catch {
    fail("send failed: \(error)")
}

var gotInlineAck = false
var gotSlotAck = false
let deadline = Date().addingTimeInterval(5)

while Date() < deadline {
    do {
        if let frame = try guest.receive() {
            if frame.id == 101, frame.payload == Array("ack-inline".utf8) {
                gotInlineAck = true
            } else if frame.id == 102, frame.payload == Array("ack-slot".utf8) {
                gotSlotAck = true
            }

            if gotInlineAck && gotSlotAck {
                guest.detach()
                print("ok")
                exit(0)
            }
        }
    } catch {
        fail("receive failed: \(error)")
    }

    usleep(10_000)
}

fail("timed out waiting for host responses")
