import Foundation

// r[impl session.message]
// r[impl session.message.payloads]
// r[impl session.connection-settings]
public struct MessageV7: Sendable, Equatable {
    public var connectionId: UInt64
    public var payload: MessagePayloadV7

    public init(connectionId: UInt64, payload: MessagePayloadV7) {
        self.connectionId = connectionId
        self.payload = payload
    }

    public func encode() -> [UInt8] {
        var out = encodeVarint(connectionId)
        out += payload.encode()
        return out
    }

    public static func decode(from data: Data) throws -> MessageV7 {
        var offset = 0
        let connectionId = try decodeVarint(from: data, offset: &offset)
        let payload = try MessagePayloadV7.decode(from: data, offset: &offset)
        guard offset == data.count else {
            throw WireV7Error.trailingBytes
        }
        return MessageV7(connectionId: connectionId, payload: payload)
    }
}

public enum WireV7Error: Error, Equatable {
    case truncated
    case unknownVariant(UInt64)
    case overflow
    case invalidUtf8
    case trailingBytes
}

public enum ParityV7: UInt64, Sendable, Equatable {
    case odd = 0
    case even = 1
}

public struct ConnectionSettingsV7: Sendable, Equatable {
    public var parity: ParityV7
    public var maxConcurrentRequests: UInt32

    public init(parity: ParityV7, maxConcurrentRequests: UInt32) {
        self.parity = parity
        self.maxConcurrentRequests = maxConcurrentRequests
    }

    fileprivate func encode() -> [UInt8] {
        encodeVarint(parity.rawValue) + encodeVarint(UInt64(maxConcurrentRequests))
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let parityRaw = try decodeVarint(from: data, offset: &offset)
        guard let parity = ParityV7(rawValue: parityRaw) else {
            throw WireV7Error.unknownVariant(parityRaw)
        }
        let maxConcurrent = try decodeVarintU32V7(from: data, offset: &offset)
        return .init(parity: parity, maxConcurrentRequests: maxConcurrent)
    }
}

public enum MetadataValueV7: Sendable, Equatable {
    case string(String)
    case bytes([UInt8])
    case u64(UInt64)

    fileprivate func encode() -> [UInt8] {
        switch self {
        case .string(let value):
            return [0] + encodeString(value)
        case .bytes(let bytes):
            return [1] + encodeBytes(bytes)
        case .u64(let value):
            return [2] + encodeVarint(value)
        }
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let disc = try decodeVarint(from: data, offset: &offset)
        switch disc {
        case 0:
            return .string(try decodeStringV7(from: data, offset: &offset))
        case 1:
            return .bytes(Array(try decodeBytesV7(from: data, offset: &offset)))
        case 2:
            return .u64(try decodeVarint(from: data, offset: &offset))
        default:
            throw WireV7Error.unknownVariant(disc)
        }
    }
}

public struct MetadataEntryV7: Sendable, Equatable {
    public var key: String
    public var value: MetadataValueV7
    public var flags: UInt64

    public init(key: String, value: MetadataValueV7, flags: UInt64) {
        self.key = key
        self.value = value
        self.flags = flags
    }
}

public typealias MetadataV7 = [MetadataEntryV7]

private func encodeMetadataV7(_ metadata: MetadataV7) -> [UInt8] {
    var out = encodeVarint(UInt64(metadata.count))
    for entry in metadata {
        out += encodeString(entry.key)
        out += entry.value.encode()
        out += encodeVarint(entry.flags)
    }
    return out
}

private func decodeMetadataV7(from data: Data, offset: inout Int) throws -> MetadataV7 {
    let count = try decodeVarint(from: data, offset: &offset)
    var result: MetadataV7 = []
    result.reserveCapacity(Int(count))
    for _ in 0..<count {
        let key = try decodeStringV7(from: data, offset: &offset)
        let value = try MetadataValueV7.decode(from: data, offset: &offset)
        let flags = try decodeVarint(from: data, offset: &offset)
        result.append(.init(key: key, value: value, flags: flags))
    }
    return result
}

public struct OpaquePayloadV7: Sendable, Equatable {
    public var bytes: [UInt8]

    public init(_ bytes: [UInt8]) {
        self.bytes = bytes
    }

    fileprivate func encode() -> [UInt8] {
        encodeBytes(bytes)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(Array(try decodeBytesV7(from: data, offset: &offset)))
    }
}

public enum MessagePayloadV7: Sendable, Equatable {
    case hello(HelloV7)
    case helloYourself(HelloYourselfV7)
    case protocolError(ProtocolErrorV7)
    case connectionOpen(ConnectionOpenV7)
    case connectionAccept(ConnectionAcceptV7)
    case connectionReject(ConnectionRejectV7)
    case connectionClose(ConnectionCloseV7)
    case requestMessage(RequestMessageV7)
    case channelMessage(ChannelMessageV7)

    fileprivate func encode() -> [UInt8] {
        switch self {
        case .hello(let hello):
            return encodeVarint(0) + hello.encode()
        case .helloYourself(let helloYourself):
            return encodeVarint(1) + helloYourself.encode()
        case .protocolError(let error):
            return encodeVarint(2) + error.encode()
        case .connectionOpen(let open):
            return encodeVarint(3) + open.encode()
        case .connectionAccept(let accept):
            return encodeVarint(4) + accept.encode()
        case .connectionReject(let reject):
            return encodeVarint(5) + reject.encode()
        case .connectionClose(let close):
            return encodeVarint(6) + close.encode()
        case .requestMessage(let request):
            return encodeVarint(7) + request.encode()
        case .channelMessage(let channel):
            return encodeVarint(8) + channel.encode()
        }
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let disc = try decodeVarint(from: data, offset: &offset)
        switch disc {
        case 0:
            return .hello(try HelloV7.decode(from: data, offset: &offset))
        case 1:
            return .helloYourself(try HelloYourselfV7.decode(from: data, offset: &offset))
        case 2:
            return .protocolError(try ProtocolErrorV7.decode(from: data, offset: &offset))
        case 3:
            return .connectionOpen(try ConnectionOpenV7.decode(from: data, offset: &offset))
        case 4:
            return .connectionAccept(try ConnectionAcceptV7.decode(from: data, offset: &offset))
        case 5:
            return .connectionReject(try ConnectionRejectV7.decode(from: data, offset: &offset))
        case 6:
            return .connectionClose(try ConnectionCloseV7.decode(from: data, offset: &offset))
        case 7:
            return .requestMessage(try RequestMessageV7.decode(from: data, offset: &offset))
        case 8:
            return .channelMessage(try ChannelMessageV7.decode(from: data, offset: &offset))
        default:
            throw WireV7Error.unknownVariant(disc)
        }
    }
}

public struct HelloV7: Sendable, Equatable {
    public var version: UInt32
    public var connectionSettings: ConnectionSettingsV7
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeVarint(UInt64(version))
            + connectionSettings.encode()
            + encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            version: try decodeVarintU32V7(from: data, offset: &offset),
            connectionSettings: try ConnectionSettingsV7.decode(from: data, offset: &offset),
            metadata: try decodeMetadataV7(from: data, offset: &offset)
        )
    }
}

public struct HelloYourselfV7: Sendable, Equatable {
    public var connectionSettings: ConnectionSettingsV7
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        connectionSettings.encode() + encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            connectionSettings: try ConnectionSettingsV7.decode(from: data, offset: &offset),
            metadata: try decodeMetadataV7(from: data, offset: &offset)
        )
    }
}

public struct ProtocolErrorV7: Sendable, Equatable {
    public var description: String

    fileprivate func encode() -> [UInt8] {
        encodeString(description)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(description: try decodeStringV7(from: data, offset: &offset))
    }
}

public struct ConnectionOpenV7: Sendable, Equatable {
    public var connectionSettings: ConnectionSettingsV7
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        connectionSettings.encode() + encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            connectionSettings: try ConnectionSettingsV7.decode(from: data, offset: &offset),
            metadata: try decodeMetadataV7(from: data, offset: &offset)
        )
    }
}

public struct ConnectionAcceptV7: Sendable, Equatable {
    public var connectionSettings: ConnectionSettingsV7
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        connectionSettings.encode() + encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            connectionSettings: try ConnectionSettingsV7.decode(from: data, offset: &offset),
            metadata: try decodeMetadataV7(from: data, offset: &offset)
        )
    }
}

public struct ConnectionRejectV7: Sendable, Equatable {
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(metadata: try decodeMetadataV7(from: data, offset: &offset))
    }
}

public struct ConnectionCloseV7: Sendable, Equatable {
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(metadata: try decodeMetadataV7(from: data, offset: &offset))
    }
}

public struct RequestMessageV7: Sendable, Equatable {
    public var id: UInt64
    public var body: RequestBodyV7

    fileprivate func encode() -> [UInt8] {
        encodeVarint(id) + body.encode()
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            id: try decodeVarint(from: data, offset: &offset),
            body: try RequestBodyV7.decode(from: data, offset: &offset)
        )
    }
}

public enum RequestBodyV7: Sendable, Equatable {
    case call(RequestCallV7)
    case response(RequestResponseV7)
    case cancel(RequestCancelV7)

    fileprivate func encode() -> [UInt8] {
        switch self {
        case .call(let value):
            return encodeVarint(0) + value.encode()
        case .response(let value):
            return encodeVarint(1) + value.encode()
        case .cancel(let value):
            return encodeVarint(2) + value.encode()
        }
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let disc = try decodeVarint(from: data, offset: &offset)
        switch disc {
        case 0:
            return .call(try RequestCallV7.decode(from: data, offset: &offset))
        case 1:
            return .response(try RequestResponseV7.decode(from: data, offset: &offset))
        case 2:
            return .cancel(try RequestCancelV7.decode(from: data, offset: &offset))
        default:
            throw WireV7Error.unknownVariant(disc)
        }
    }
}

public struct RequestCallV7: Sendable, Equatable {
    public var methodId: UInt64
    public var args: OpaquePayloadV7
    public var channels: [UInt64]
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        var out = encodeVarint(methodId)
        out += args.encode()
        out += encodeVarint(UInt64(channels.count))
        for channel in channels {
            out += encodeVarint(channel)
        }
        out += encodeMetadataV7(metadata)
        return out
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let methodId = try decodeVarint(from: data, offset: &offset)
        let args = try OpaquePayloadV7.decode(from: data, offset: &offset)
        let channelCount = try decodeVarint(from: data, offset: &offset)
        var channels: [UInt64] = []
        channels.reserveCapacity(Int(channelCount))
        for _ in 0..<channelCount {
            channels.append(try decodeVarint(from: data, offset: &offset))
        }
        let metadata = try decodeMetadataV7(from: data, offset: &offset)
        return .init(methodId: methodId, args: args, channels: channels, metadata: metadata)
    }
}

public struct RequestResponseV7: Sendable, Equatable {
    public var ret: OpaquePayloadV7
    public var channels: [UInt64]
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        var out = ret.encode()
        out += encodeVarint(UInt64(channels.count))
        for channel in channels {
            out += encodeVarint(channel)
        }
        out += encodeMetadataV7(metadata)
        return out
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let ret = try OpaquePayloadV7.decode(from: data, offset: &offset)
        let channelCount = try decodeVarint(from: data, offset: &offset)
        var channels: [UInt64] = []
        channels.reserveCapacity(Int(channelCount))
        for _ in 0..<channelCount {
            channels.append(try decodeVarint(from: data, offset: &offset))
        }
        let metadata = try decodeMetadataV7(from: data, offset: &offset)
        return .init(ret: ret, channels: channels, metadata: metadata)
    }
}

public struct RequestCancelV7: Sendable, Equatable {
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(metadata: try decodeMetadataV7(from: data, offset: &offset))
    }
}

public struct ChannelMessageV7: Sendable, Equatable {
    public var id: UInt64
    public var body: ChannelBodyV7

    fileprivate func encode() -> [UInt8] {
        encodeVarint(id) + body.encode()
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(
            id: try decodeVarint(from: data, offset: &offset),
            body: try ChannelBodyV7.decode(from: data, offset: &offset)
        )
    }
}

public enum ChannelBodyV7: Sendable, Equatable {
    case item(ChannelItemV7)
    case close(ChannelCloseV7)
    case reset(ChannelResetV7)
    case grantCredit(ChannelGrantCreditV7)

    fileprivate func encode() -> [UInt8] {
        switch self {
        case .item(let item):
            return encodeVarint(0) + item.encode()
        case .close(let close):
            return encodeVarint(1) + close.encode()
        case .reset(let reset):
            return encodeVarint(2) + reset.encode()
        case .grantCredit(let credit):
            return encodeVarint(3) + credit.encode()
        }
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        let disc = try decodeVarint(from: data, offset: &offset)
        switch disc {
        case 0:
            return .item(try ChannelItemV7.decode(from: data, offset: &offset))
        case 1:
            return .close(try ChannelCloseV7.decode(from: data, offset: &offset))
        case 2:
            return .reset(try ChannelResetV7.decode(from: data, offset: &offset))
        case 3:
            return .grantCredit(try ChannelGrantCreditV7.decode(from: data, offset: &offset))
        default:
            throw WireV7Error.unknownVariant(disc)
        }
    }
}

public struct ChannelItemV7: Sendable, Equatable {
    public var item: OpaquePayloadV7

    fileprivate func encode() -> [UInt8] {
        item.encode()
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(item: try OpaquePayloadV7.decode(from: data, offset: &offset))
    }
}

public struct ChannelCloseV7: Sendable, Equatable {
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(metadata: try decodeMetadataV7(from: data, offset: &offset))
    }
}

public struct ChannelResetV7: Sendable, Equatable {
    public var metadata: MetadataV7

    fileprivate func encode() -> [UInt8] {
        encodeMetadataV7(metadata)
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(metadata: try decodeMetadataV7(from: data, offset: &offset))
    }
}

public struct ChannelGrantCreditV7: Sendable, Equatable {
    public var additional: UInt32

    fileprivate func encode() -> [UInt8] {
        encodeVarint(UInt64(additional))
    }

    fileprivate static func decode(from data: Data, offset: inout Int) throws -> Self {
        .init(additional: try decodeVarintU32V7(from: data, offset: &offset))
    }
}

@inline(__always)
private func decodeVarintU32V7(from data: Data, offset: inout Int) throws -> UInt32 {
    let value = try decodeVarint(from: data, offset: &offset)
    guard value <= UInt64(UInt32.max) else {
        throw WireV7Error.overflow
    }
    return UInt32(value)
}

@inline(__always)
private func decodeStringV7(from data: Data, offset: inout Int) throws -> String {
    do {
        return try decodeString(from: data, offset: &offset)
    } catch PostcardError.invalidUtf8 {
        throw WireV7Error.invalidUtf8
    } catch PostcardError.truncated {
        throw WireV7Error.truncated
    } catch {
        throw error
    }
}

@inline(__always)
private func decodeBytesV7(from data: Data, offset: inout Int) throws -> Data {
    do {
        return try decodeBytes(from: data, offset: &offset)
    } catch PostcardError.truncated {
        throw WireV7Error.truncated
    } catch {
        throw error
    }
}

public extension MessageV7 {
    static func hello(_ hello: HelloV7) -> MessageV7 {
        MessageV7(connectionId: 0, payload: .hello(hello))
    }

    static func helloYourself(_ hello: HelloYourselfV7) -> MessageV7 {
        MessageV7(connectionId: 0, payload: .helloYourself(hello))
    }

    static func protocolError(description: String) -> MessageV7 {
        MessageV7(connectionId: 0, payload: .protocolError(.init(description: description)))
    }

    static func connectionOpen(connId: UInt64, settings: ConnectionSettingsV7, metadata: MetadataV7) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .connectionOpen(.init(connectionSettings: settings, metadata: metadata))
        )
    }

    static func connectionAccept(connId: UInt64, settings: ConnectionSettingsV7, metadata: MetadataV7) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .connectionAccept(.init(connectionSettings: settings, metadata: metadata))
        )
    }

    static func connectionReject(connId: UInt64, metadata: MetadataV7) -> MessageV7 {
        MessageV7(connectionId: connId, payload: .connectionReject(.init(metadata: metadata)))
    }

    static func connectionClose(connId: UInt64, metadata: MetadataV7) -> MessageV7 {
        MessageV7(connectionId: connId, payload: .connectionClose(.init(metadata: metadata)))
    }

    static func request(
        connId: UInt64,
        requestId: UInt64,
        methodId: UInt64,
        metadata: MetadataV7,
        channels: [UInt64],
        payload: [UInt8]
    ) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .requestMessage(
                .init(
                    id: requestId,
                    body: .call(.init(methodId: methodId, args: .init(payload), channels: channels, metadata: metadata))
                ))
        )
    }

    static func response(
        connId: UInt64,
        requestId: UInt64,
        metadata: MetadataV7,
        channels: [UInt64],
        payload: [UInt8]
    ) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .requestMessage(
                .init(
                    id: requestId,
                    body: .response(.init(ret: .init(payload), channels: channels, metadata: metadata))
                ))
        )
    }

    static func cancel(connId: UInt64, requestId: UInt64, metadata: MetadataV7 = []) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .requestMessage(
                .init(
                    id: requestId,
                    body: .cancel(.init(metadata: metadata))
                ))
        )
    }

    static func data(connId: UInt64, channelId: UInt64, payload: [UInt8]) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .channelMessage(.init(id: channelId, body: .item(.init(item: .init(payload)))))
        )
    }

    static func close(connId: UInt64, channelId: UInt64, metadata: MetadataV7 = []) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .channelMessage(.init(id: channelId, body: .close(.init(metadata: metadata))))
        )
    }

    static func reset(connId: UInt64, channelId: UInt64, metadata: MetadataV7 = []) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .channelMessage(.init(id: channelId, body: .reset(.init(metadata: metadata))))
        )
    }

    static func credit(connId: UInt64, channelId: UInt64, bytes: UInt32) -> MessageV7 {
        MessageV7(
            connectionId: connId,
            payload: .channelMessage(.init(id: channelId, body: .grantCredit(.init(additional: bytes))))
        )
    }
}
