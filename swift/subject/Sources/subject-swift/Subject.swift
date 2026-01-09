/// Swift subject binary for the roam compliance suite.
///
/// This demonstrates the minimal code needed to implement a roam service
/// using the roam-runtime transport library.

import Dispatch
import Foundation
import RoamRuntime

// MARK: - Testbed Service Implementation

/// Implementation of the Testbed service.
struct TestbedService: TestbedHandler {

    // ========================================================================
    // Unary methods
    // ========================================================================

    func echo(message: String) async throws -> String {
        log("echo called: \(message)")
        return message
    }

    func reverse(message: String) async throws -> String {
        log("reverse called: \(message)")
        return String(message.reversed())
    }

    // ========================================================================
    // Streaming methods
    // ========================================================================

    func sum(numbers: Rx<Int32>) async throws -> Int64 {
        log("sum called, starting to receive numbers")
        var total: Int64 = 0
        for try await n in numbers {
            log("  received: \(n)")
            total += Int64(n)
        }
        log("sum complete: \(total)")
        return total
    }

    func generate(count: UInt32, output: Tx<Int32>) async throws {
        log("generate called: count=\(count)")
        for i in 0..<Int32(count) {
            log("  sending: \(i)")
            try output.send(i)
        }
        log("generate complete")
    }

    func transform(input: Rx<String>, output: Tx<String>) async throws {
        log("transform called")
        for try await s in input {
            log("  transforming: \(s)")
            try output.send(s)
        }
        log("transform complete")
    }

    // ========================================================================
    // Complex type methods
    // ========================================================================

    func echoPoint(point: Point) async throws -> Point {
        return point
    }

    func createPerson(name: String, age: UInt8, email: String?) async throws -> Person {
        return Person(name: name, age: age, email: email)
    }

    func rectangleArea(rect: Rectangle) async throws -> Double {
        let width = abs(Double(rect.bottomRight.x - rect.topLeft.x))
        let height = abs(Double(rect.bottomRight.y - rect.topLeft.y))
        return width * height
    }

    func parseColor(name: String) async throws -> Color? {
        switch name.lowercased() {
        case "red": return .red
        case "green": return .green
        case "blue": return .blue
        default: return nil
        }
    }

    func shapeArea(shape: Shape) async throws -> Double {
        switch shape {
        case .circle(let radius):
            return Double.pi * radius * radius
        case .rectangle(let width, let height):
            return width * height
        case .point:
            return 0.0
        }
    }

    func createCanvas(name: String, shapes: [Shape], background: Color) async throws -> Canvas {
        return Canvas(name: name, shapes: shapes, background: background)
    }

    func processMessage(msg: Message) async throws -> Message {
        switch msg {
        case .text(let s):
            return .text("processed: \(s)")
        case .number(let n):
            return .number(n * 2)
        case .data(let d):
            return .data(Data(d.reversed()))
        }
    }

    func getPoints(count: UInt32) async throws -> [Point] {
        return (0..<Int32(count)).map { Point(x: $0, y: $0 * 2) }
    }

    func swapPair(pair: (Int32, String)) async throws -> (String, Int32) {
        return (pair.1, pair.0)
    }
}

// MARK: - Streaming Dispatcher Adapter

/// Adapter to make TestbedStreamingDispatcher conform to ServiceDispatcher.
final class TestbedDispatcherAdapter: ServiceDispatcher, @unchecked Sendable {
    private let handler: TestbedHandler

    init(handler: TestbedHandler) {
        self.handler = handler
    }

    func dispatch(
        methodId: UInt64,
        payload: [UInt8],
        requestId: UInt64,
        registry: ChannelRegistry,
        taskTx: @escaping @Sendable (TaskMessage) -> Void
    ) async {
        let dispatcher = TestbedStreamingDispatcher(
            handler: handler,
            registry: registry,
            taskSender: taskTx
        )

        // Preregister channels synchronously
        await dispatcher.preregisterChannels(methodId: methodId, payload: Data(payload))

        // Dispatch the request
        await dispatcher.dispatch(methodId: methodId, requestId: requestId, payload: Data(payload))
    }
}

// MARK: - Logging

func log(_ message: String) {
    FileHandle.standardError.write(
        "[\(ProcessInfo.processInfo.processIdentifier)] \(message)\n".data(using: .utf8)!)
}

// MARK: - Server Mode

/// In "server" mode, the subject acts as the RPC server (handler).
/// But it CONNECTS TO the test harness (specified by PEER_ADDR).
/// This is confusing terminology but matches how the Rust subject works.
func runServer() async throws {
    guard let addr = ProcessInfo.processInfo.environment["PEER_ADDR"] else {
        throw SubjectError.connectFailed
    }

    log("connecting to \(addr)")

    // Parse host:port
    let parts = addr.split(separator: ":")
    guard parts.count == 2,
        let port = UInt16(parts[1])
    else {
        throw SubjectError.connectFailed
    }
    let host = String(parts[0])

    // Create socket
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else {
        throw SubjectError.socketCreationFailed
    }

    // Connect
    var serverAddr = sockaddr_in()
    serverAddr.sin_family = sa_family_t(AF_INET)
    serverAddr.sin_port = port.bigEndian
    inet_pton(AF_INET, host, &serverAddr.sin_addr)

    let connectResult = withUnsafePointer(to: &serverAddr) { addrPtr in
        addrPtr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
            Darwin.connect(fd, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard connectResult == 0 else {
        close(fd)
        throw SubjectError.connectFailed
    }

    log("connected")

    // Create transport
    let transport = SocketTransport(fd: fd)

    // Establish connection as acceptor (we're the server/handler, but we connected)
    let hello = Hello.v1(maxPayloadSize: 1024 * 1024, initialChannelCredit: 64 * 1024)
    let handler = TestbedService()
    let dispatcher = TestbedDispatcherAdapter(handler: handler)

    let (_, driver) = try await establishAcceptor(
        transport: transport,
        ourHello: hello,
        dispatcher: dispatcher
    )

    log("handshake complete, running driver")

    // Run driver
    try await driver.run()

    log("driver finished")
}

// MARK: - Socket Transport

/// Simple synchronous socket transport for stdio-based testing.
final class SocketTransport: MessageTransport, @unchecked Sendable {
    private let fd: Int32
    private var readBuffer: [UInt8] = []

    init(fd: Int32) {
        self.fd = fd
    }

    func send(_ message: RoamRuntime.Message) async throws {
        let encoded = message.encode()
        var framed = cobsEncode(encoded)
        framed.append(0)  // Frame delimiter

        var written = 0
        while written < framed.count {
            let result = Darwin.write(fd, &framed[written], framed.count - written)
            if result < 0 {
                throw TransportError.connectionClosed
            }
            written += result
        }
    }

    func recv() async throws -> RoamRuntime.Message? {
        // Read until we get a complete frame (zero delimiter)
        while true {
            if let zeroIndex = readBuffer.firstIndex(of: 0) {
                let frame = Array(readBuffer[..<zeroIndex])
                readBuffer.removeFirst(zeroIndex + 1)

                if frame.isEmpty {
                    continue
                }

                let decoded = try cobsDecode(frame)
                return try RoamRuntime.Message.decode(from: Data(decoded))
            }

            // Read more data
            var chunk = [UInt8](repeating: 0, count: 4096)
            let result = Darwin.read(fd, &chunk, chunk.count)
            if result <= 0 {
                return nil  // EOF
            }
            readBuffer.append(contentsOf: chunk[..<result])
        }
    }

    func close() async throws {
        Darwin.close(fd)
    }
}

// MARK: - Errors

enum SubjectError: Error {
    case socketCreationFailed
    case bindFailed
    case listenFailed
    case acceptFailed
    case connectFailed
}

// MARK: - Main Entry Point

@main
struct SubjectMain {
    static func main() async {
        let mode = ProcessInfo.processInfo.environment["SUBJECT_MODE"] ?? "server"
        log("subject-swift starting in \(mode) mode")

        do {
            switch mode {
            case "server":
                try await runServer()
            case "client":
                try await runClient()
            default:
                log("unknown SUBJECT_MODE: \(mode)")
                exit(1)
            }
        } catch {
            log("error: \(error)")
            exit(1)
        }
    }
}

// MARK: - Client Mode

func runClient() async throws {
    guard let addr = ProcessInfo.processInfo.environment["PEER_ADDR"] else {
        throw SubjectError.connectFailed
    }

    log("connecting to \(addr)")

    // Parse host:port
    let parts = addr.split(separator: ":")
    guard parts.count == 2,
        let port = UInt16(parts[1])
    else {
        throw SubjectError.connectFailed
    }
    let host = String(parts[0])

    // Create socket
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else {
        throw SubjectError.socketCreationFailed
    }

    // Connect
    var serverAddr = sockaddr_in()
    serverAddr.sin_family = sa_family_t(AF_INET)
    serverAddr.sin_port = port.bigEndian
    inet_pton(AF_INET, host, &serverAddr.sin_addr)

    let connectResult = withUnsafePointer(to: &serverAddr) { addrPtr in
        addrPtr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
            Darwin.connect(fd, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard connectResult == 0 else {
        close(fd)
        throw SubjectError.connectFailed
    }

    log("connected")

    // Create transport
    let transport = SocketTransport(fd: fd)

    // Establish connection as initiator
    let hello = Hello.v1(maxPayloadSize: 1024 * 1024, initialChannelCredit: 64 * 1024)
    let handler = TestbedService()
    let dispatcher = TestbedDispatcherAdapter(handler: handler)

    let (handle, driver) = try await establishInitiator(
        transport: transport,
        ourHello: hello,
        dispatcher: dispatcher
    )

    log("handshake complete")

    // Spawn driver
    Task {
        do {
            try await driver.run()
        } catch {
            log("driver error: \(error)")
        }
    }

    // Create client
    let client = TestbedClient(connection: handle)

    // Run test scenario
    let scenario = ProcessInfo.processInfo.environment["CLIENT_SCENARIO"] ?? "echo"
    log("running client scenario: \(scenario)")

    switch scenario {
    case "echo":
        let result = try await client.echo(message: "hello from swift")
        log("echo result: \(result)")

    default:
        log("unknown CLIENT_SCENARIO: \(scenario)")
    }
}
