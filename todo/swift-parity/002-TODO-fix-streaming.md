# Phase 002: Fix Streaming RPC

**Status**: TODO

## Problem

Streaming tests fail because Response is sent before Data/Close messages:

```
[WIRE] --> Request { id: 1, method: 0x54d2273d8cdb9c38, payload: 2 bytes }
[WIRE] <-- Response { id: 1, payload: 2 bytes }
# Missing: Data messages and Close!
```

The spec requires for server-to-client streaming:
1. Data messages (0..N)
2. Close message
3. Response message

## Root Cause

In `dispatchgenerate`:
```swift
let output = createServerTx(channelId: outputChannelId, taskSender: taskSender, ...)
try await handler.generate(count: count, output: output)  // Calls output.send(i) 
output.close()  // Yields Close to event stream
taskSender(.response(...))  // Yields Response to event stream
```

The problem is that `Tx.send()` and `Tx.close()` just yield to an `AsyncStream`:
```swift
public func send(_ value: T) throws {
    taskTx(.data(channelId: channelId, payload: serialize(value)))  // Fire and forget!
}
```

The `taskTx` closure yields to `eventContinuation`, but the dispatcher doesn't wait for these messages to be processed before sending Response.

## Comparison with TypeScript

TypeScript dispatcher awaits each send:
```typescript
await output.send(value);  // Actually waits for send to complete
```

## Solution Options

### Option A: Make Tx.send() async

Change `Tx.send()` to be async and actually wait for the message to be sent:

```swift
public func send(_ value: T) async throws {
    guard let taskTx = taskTx else { throw ChannelError.notBound }
    let bytes = serialize(value)
    await taskTx(.data(channelId: channelId, payload: bytes))
}
```

This requires:
1. Change `TaskSender` to be async: `@Sendable (TaskMessage) async -> Void`
2. Change all dispatch methods to use `try await output.send(i)`
3. Update generated code in `roam-codegen`

**Pros**: Cleaner semantics, matches TypeScript
**Cons**: Requires async TaskSender, more code changes

### Option B: Flush pending messages before Response

Add a mechanism to wait for all pending task messages to be sent:

```swift
// In dispatcher, before sending response:
await flushPendingMessages()
taskSender(.response(...))
```

**Pros**: Minimal changes to Tx API
**Cons**: Adds complexity to driver

### Option C: Use channel with acknowledgment

Use a proper async channel that provides backpressure:

```swift
let (stream, continuation) = AsyncStream.makeStream(of: TaskMessage.self)
// continuation.yield() already provides backpressure in Swift 5.9+
```

**Pros**: Proper async semantics
**Cons**: May require Swift 5.9+ features

## Recommended Approach

**Option A** - Make `Tx.send()` async. This matches the TypeScript pattern and provides correct semantics.

## Tasks

1. [ ] Change `TaskSender` to async signature
2. [ ] Update `Tx.send()` to be async
3. [ ] Update `Tx.close()` to be async  
4. [ ] Update Driver to handle async task messages
5. [ ] Update `roam-codegen` server.rs to generate `try await output.send()`
6. [ ] Update `roam-codegen` server.rs to generate `await output.close()`
7. [ ] Rebuild swift subject: `cd swift/subject && swift build -c release`
8. [ ] Run streaming tests: `SUBJECT_CMD="./swift/subject/.build/release/subject-swift" cargo test -p spec-tests streaming`
9. [ ] Verify all 3 streaming tests pass

## Success Criteria

All streaming tests pass:
- `streaming_sum_client_to_server`
- `streaming_generate_server_to_client`
- `streaming_transform_bidirectional`

## Files to Modify

### Swift Runtime
- `swift/roam-runtime/Sources/RoamRuntime/Channel.swift` - Tx.send/close async
- `swift/roam-runtime/Sources/RoamRuntime/Binding.swift` - TaskSender signature
- `swift/roam-runtime/Sources/RoamRuntime/Driver.swift` - Handle async task messages
- `swift/roam-runtime/Sources/RoamRuntime/RoamRuntime.swift` - createServerTx

### Rust Codegen
- `rust/roam-codegen/src/targets/swift/server.rs` - Generate await for send/close
