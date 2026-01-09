import Foundation

// MARK: - Negotiated Parameters

/// Parameters negotiated during handshake.
public struct Negotiated: Sendable {
    public let maxPayloadSize: UInt32
    public let initialCredit: UInt32

    public init(maxPayloadSize: UInt32, initialCredit: UInt32) {
        self.maxPayloadSize = maxPayloadSize
        self.initialCredit = initialCredit
    }
}

// MARK: - Service Dispatcher Protocol

/// Protocol for dispatching incoming requests.
public protocol ServiceDispatcher: Sendable {
    /// Dispatch a request. Returns a future that sends the response.
    func dispatch(
        methodId: UInt64,
        payload: [UInt8],
        requestId: UInt64,
        registry: ChannelRegistry,
        taskTx: @escaping @Sendable (TaskMessage) -> Void
    ) async
}

// MARK: - Handle Command

/// Commands from ConnectionHandle to Driver.
enum HandleCommand: Sendable {
    case call(
        requestId: UInt64,
        methodId: UInt64,
        payload: [UInt8],
        responseTx: @Sendable (Result<[UInt8], ConnectionError>) -> Void
    )
}

// MARK: - Connection Handle

/// Handle for making outgoing RPC calls.
public final class ConnectionHandle: @unchecked Sendable {
    private let commandTx: @Sendable (HandleCommand) -> Void
    private var nextRequestId: UInt64 = 1
    private let lock = NSLock()

    public let channelAllocator: ChannelIdAllocator
    public let channelRegistry: ChannelRegistry

    init(
        commandTx: @escaping @Sendable (HandleCommand) -> Void,
        role: Role
    ) {
        self.commandTx = commandTx
        self.channelAllocator = ChannelIdAllocator(role: role)
        self.channelRegistry = ChannelRegistry()
    }

    /// Make a raw RPC call.
    public func callRaw(methodId: UInt64, payload: [UInt8]) async throws -> [UInt8] {
        let requestId = allocateRequestId()

        return try await withCheckedThrowingContinuation { cont in
            let responseTx: @Sendable (Result<[UInt8], ConnectionError>) -> Void = { result in
                cont.resume(with: result)
            }
            commandTx(
                .call(
                    requestId: requestId,
                    methodId: methodId,
                    payload: payload,
                    responseTx: responseTx
                ))
        }
    }

    private func allocateRequestId() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        let id = nextRequestId
        nextRequestId += 1
        return id
    }
}

// MARK: - Driver

/// Bidirectional connection driver.
public actor Driver {
    private let transport: any MessageTransport
    private let dispatcher: any ServiceDispatcher
    private let role: Role
    private let negotiated: Negotiated

    private let handle: ConnectionHandle
    private var commandQueue: [(HandleCommand, CheckedContinuation<Void, Never>)] = []

    private let serverRegistry: ChannelRegistry
    private var pendingResponses: [UInt64: @Sendable (Result<[UInt8], ConnectionError>) -> Void] =
        [:]
    private var inFlightRequests: Set<UInt64> = []

    private var taskQueue: [TaskMessage] = []

    public init(
        transport: any MessageTransport,
        dispatcher: any ServiceDispatcher,
        role: Role,
        negotiated: Negotiated,
        handle: ConnectionHandle
    ) {
        self.transport = transport
        self.dispatcher = dispatcher
        self.role = role
        self.negotiated = negotiated
        self.handle = handle
        self.serverRegistry = ChannelRegistry()
    }

    /// Run the driver until connection closes.
    public func run() async throws {
        while true {
            // Process any pending task messages first
            while !taskQueue.isEmpty {
                let msg = taskQueue.removeFirst()
                try await handleTaskMessage(msg)
            }

            // Then check for incoming messages
            guard let message = try await transport.recv() else {
                // EOF - clean shutdown
                failAllPending()
                return
            }

            try await handleMessage(message)
        }
    }

    /// Queue a command from the handle.
    func enqueueCommand(_ cmd: HandleCommand) {
        switch cmd {
        case .call(let requestId, let methodId, let payload, let responseTx):
            pendingResponses[requestId] = responseTx
            Task {
                let msg = Message.request(
                    requestId: requestId,
                    methodId: methodId,
                    metadata: [],
                    payload: payload
                )
                try? await transport.send(msg)
            }
        }
    }

    /// Queue a task message (Data/Close/Response from handlers).
    func enqueueTaskMessage(_ msg: TaskMessage) {
        taskQueue.append(msg)
    }

    private func handleTaskMessage(_ msg: TaskMessage) async throws {
        let wireMsg: Message
        switch msg {
        case .data(let channelId, let payload):
            wireMsg = .data(channelId: channelId, payload: payload)
        case .close(let channelId):
            wireMsg = .close(channelId: channelId)
        case .response(let requestId, let payload):
            guard inFlightRequests.remove(requestId) != nil else {
                return  // Already cancelled
            }
            wireMsg = .response(requestId: requestId, metadata: [], payload: payload)
        }
        try await transport.send(wireMsg)
    }

    private func handleMessage(_ msg: Message) async throws {
        switch msg {
        case .hello:
            // Duplicate hello, ignore
            break

        case .goodbye(let reason):
            failAllPending()
            throw ConnectionError.goodbye(reason: reason)

        case .request(let requestId, let methodId, _, let payload):
            try await handleRequest(requestId: requestId, methodId: methodId, payload: payload)

        case .response(let requestId, _, let payload):
            if let responseTx = pendingResponses.removeValue(forKey: requestId) {
                responseTx(.success(payload))
            }

        case .cancel:
            // TODO: implement cancellation
            break

        case .data(let channelId, let payload):
            try await handleData(channelId: channelId, payload: payload)

        case .close(let channelId):
            try await handleClose(channelId: channelId)

        case .reset(let channelId):
            // TODO: handle reset
            _ = channelId
            break

        case .credit:
            // TODO: handle credit
            break
        }
    }

    private func handleRequest(requestId: UInt64, methodId: UInt64, payload: [UInt8]) async throws {
        // Check for duplicate
        guard inFlightRequests.insert(requestId).inserted else {
            try await sendGoodbye("unary.request-id.duplicate-detection")
            throw ConnectionError.protocolViolation(rule: "unary.request-id.duplicate-detection")
        }

        // Validate payload size
        if payload.count > Int(negotiated.maxPayloadSize) {
            try await sendGoodbye("flow.unary.payload-limit")
            throw ConnectionError.protocolViolation(rule: "flow.unary.payload-limit")
        }

        // Create task sender that queues to driver
        let taskTx: @Sendable (TaskMessage) -> Void = { [weak self] msg in
            Task { [weak self] in
                await self?.enqueueTaskMessage(msg)
            }
        }

        // Dispatch (spawns handler task)
        Task {
            await dispatcher.dispatch(
                methodId: methodId,
                payload: payload,
                requestId: requestId,
                registry: serverRegistry,
                taskTx: taskTx
            )
        }
    }

    private func handleData(channelId: UInt64, payload: [UInt8]) async throws {
        if channelId == 0 {
            try await sendGoodbye("streaming.id.zero-reserved")
            throw ConnectionError.protocolViolation(rule: "streaming.id.zero-reserved")
        }

        // Try server registry first, then client registry
        var delivered = await serverRegistry.deliverData(channelId: channelId, payload: payload)
        if !delivered {
            delivered = await handle.channelRegistry.deliverData(
                channelId: channelId, payload: payload)
        }

        if !delivered {
            try await sendGoodbye("streaming.unknown")
            throw ConnectionError.protocolViolation(rule: "streaming.unknown")
        }
    }

    private func handleClose(channelId: UInt64) async throws {
        if channelId == 0 {
            try await sendGoodbye("streaming.id.zero-reserved")
            throw ConnectionError.protocolViolation(rule: "streaming.id.zero-reserved")
        }

        var delivered = await serverRegistry.deliverClose(channelId: channelId)
        if !delivered {
            delivered = await handle.channelRegistry.deliverClose(channelId: channelId)
        }

        if !delivered {
            try await sendGoodbye("streaming.unknown")
            throw ConnectionError.protocolViolation(rule: "streaming.unknown")
        }
    }

    private func sendGoodbye(_ reason: String) async throws {
        try await transport.send(.goodbye(reason: reason))
    }

    private func failAllPending() {
        for (_, responseTx) in pendingResponses {
            responseTx(.failure(.connectionClosed))
        }
        pendingResponses.removeAll()
    }
}

// MARK: - Connection Errors

public enum ConnectionError: Error {
    case connectionClosed
    case goodbye(reason: String)
    case protocolViolation(rule: String)
    case handshakeFailed(String)
}

// MARK: - Establish Connection

/// Establish a connection as initiator.
public func establishInitiator(
    transport: any MessageTransport,
    ourHello: Hello,
    dispatcher: any ServiceDispatcher
) async throws -> (ConnectionHandle, Driver) {
    // Send our hello
    try await transport.send(.hello(ourHello))

    // Wait for peer hello
    guard let peerMsg = try await transport.recv(),
        case .hello(let peerHello) = peerMsg
    else {
        throw ConnectionError.handshakeFailed("expected Hello")
    }

    let (ourMax, ourCredit) =
        switch ourHello {
        case .v1(let max, let credit): (max, credit)
        }
    let (peerMax, peerCredit) =
        switch peerHello {
        case .v1(let max, let credit): (max, credit)
        }

    let negotiated = Negotiated(
        maxPayloadSize: min(ourMax, peerMax),
        initialCredit: min(ourCredit, peerCredit)
    )

    // Create handle with command channel
    var enqueueCommand: (@Sendable (HandleCommand) -> Void)!
    let handle = ConnectionHandle(
        commandTx: { cmd in enqueueCommand(cmd) },
        role: .initiator
    )

    let driver = Driver(
        transport: transport,
        dispatcher: dispatcher,
        role: .initiator,
        negotiated: negotiated,
        handle: handle
    )

    enqueueCommand = { cmd in
        Task {
            await driver.enqueueCommand(cmd)
        }
    }

    return (handle, driver)
}

/// Establish a connection as acceptor.
public func establishAcceptor(
    transport: any MessageTransport,
    ourHello: Hello,
    dispatcher: any ServiceDispatcher
) async throws -> (ConnectionHandle, Driver) {
    // Send our hello immediately
    try await transport.send(.hello(ourHello))

    // Wait for peer hello
    guard let peerMsg = try await transport.recv(),
        case .hello(let peerHello) = peerMsg
    else {
        throw ConnectionError.handshakeFailed("expected Hello")
    }

    let (ourMax, ourCredit) =
        switch ourHello {
        case .v1(let max, let credit): (max, credit)
        }
    let (peerMax, peerCredit) =
        switch peerHello {
        case .v1(let max, let credit): (max, credit)
        }

    let negotiated = Negotiated(
        maxPayloadSize: min(ourMax, peerMax),
        initialCredit: min(ourCredit, peerCredit)
    )

    // Create handle with command channel
    var enqueueCommand: (@Sendable (HandleCommand) -> Void)!
    let handle = ConnectionHandle(
        commandTx: { cmd in enqueueCommand(cmd) },
        role: .acceptor
    )

    let driver = Driver(
        transport: transport,
        dispatcher: dispatcher,
        role: .acceptor,
        negotiated: negotiated,
        handle: handle
    )

    enqueueCommand = { cmd in
        Task {
            await driver.enqueueCommand(cmd)
        }
    }

    return (handle, driver)
}
