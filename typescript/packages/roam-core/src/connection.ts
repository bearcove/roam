// Connection state machine and message loop.
//
// Handles the protocol state machine including Hello exchange,
// payload validation, and stream ID management.
//
// Generic over MessageTransport to support different transports:
// - CobsFramed for TCP (byte streams with COBS framing)
// - WsTransport for WebSocket (message-oriented transport)

import { concat, encodeBytes, encodeString } from "../../roam-postcard/src/binary/bytes.ts";
import {
  decodeVarint,
  decodeVarintNumber,
  encodeVarint,
} from "../../roam-postcard/src/binary/varint.ts";
import {
  ChannelRegistry,
  ChannelIdAllocator,
  ChannelError,
  Role,
  type TaskMessage,
  type TaskSender,
} from "./channeling/index.ts";
import { type MessageTransport } from "./transport.ts";

// Note: Role is exported from streaming/index.ts in roam-core's main export

/** Negotiated connection parameters after Hello exchange. */
export interface Negotiated {
  /** Effective max payload size (min of both peers). */
  maxPayloadSize: number;
  /** Initial stream credit (min of both peers). */
  initialCredit: number;
}

/** Error during connection handling. */
export class ConnectionError extends Error {
  constructor(
    public kind: "io" | "protocol" | "dispatch" | "closed",
    message: string,
    public ruleId?: string,
  ) {
    super(message);
    this.name = "ConnectionError";
  }

  static io(message: string): ConnectionError {
    return new ConnectionError("io", message);
  }

  static protocol(ruleId: string, context: string): ConnectionError {
    return new ConnectionError("protocol", context, ruleId);
  }

  static dispatch(message: string): ConnectionError {
    return new ConnectionError("dispatch", message);
  }

  static closed(): ConnectionError {
    return new ConnectionError("closed", "connection closed");
  }
}

/** Hello message version 1. */
interface HelloV1 {
  variant: 0;
  maxPayloadSize: number;
  initialStreamCredit: number;
}

type Hello = HelloV1;

/** Message discriminants. */
const MSG_HELLO = 0;
const MSG_GOODBYE = 1;
const MSG_REQUEST = 2;
const MSG_RESPONSE = 3;
// const MSG_CANCEL = 4;
const MSG_DATA = 5;
const MSG_CLOSE = 6;
const MSG_RESET = 7;
const MSG_CREDIT = 8;

/** Encode a Hello message. */
function encodeHello(hello: Hello): Uint8Array {
  return concat(
    encodeVarint(MSG_HELLO),
    encodeVarint(hello.variant),
    encodeVarint(hello.maxPayloadSize),
    encodeVarint(hello.initialStreamCredit),
  );
}

/** Encode a Goodbye message. */
function encodeGoodbye(reason: string): Uint8Array {
  return concat(encodeVarint(MSG_GOODBYE), encodeString(reason));
}

/** Encode a Response message. */
function encodeResponse(requestId: bigint, payload: Uint8Array): Uint8Array {
  return concat(
    encodeVarint(MSG_RESPONSE),
    encodeVarint(requestId),
    encodeVarint(0), // empty metadata vec
    encodeBytes(payload),
  );
}

/** Encode a Request message. */
function encodeRequest(requestId: bigint, methodId: bigint, payload: Uint8Array): Uint8Array {
  return concat(
    encodeVarint(MSG_REQUEST),
    encodeVarint(requestId),
    encodeVarint(methodId),
    encodeVarint(0), // empty metadata vec
    encodeBytes(payload),
  );
}

/** Encode a Data message. */
function encodeData(streamId: bigint, payload: Uint8Array): Uint8Array {
  return concat(encodeVarint(MSG_DATA), encodeVarint(streamId), encodeBytes(payload));
}

/** Encode a Close message. */
function encodeClose(streamId: bigint): Uint8Array {
  return concat(encodeVarint(MSG_CLOSE), encodeVarint(streamId));
}

/** Trait for dispatching unary requests to a service. */
export interface ServiceDispatcher {
  /**
   * Dispatch a unary request and return the response payload.
   *
   * The dispatcher is responsible for:
   * - Looking up the method by method_id
   * - Deserializing arguments from payload
   * - Calling the service method
   * - Serializing the response
   */
  dispatchUnary(methodId: bigint, payload: Uint8Array): Promise<Uint8Array>;
}

/**
 * Streaming-aware service dispatcher.
 *
 * Unlike ServiceDispatcher which returns a response directly, this interface
 * sends responses (and streaming Data/Close messages) via a TaskSender callback.
 * This ensures proper ordering: all Data/Close messages are sent before Response.
 */
export interface StreamingDispatcher {
  /**
   * Dispatch a request that may involve streaming.
   *
   * The dispatcher is responsible for:
   * - Looking up the method by method_id
   * - Deserializing arguments from payload
   * - Creating Rx/Tx handles for stream arguments using the registry
   * - Calling the service method
   * - Sending Data/Close messages for any Tx streams via taskSender
   * - Sending the Response message via taskSender when done
   *
   * @param methodId - The method ID to dispatch
   * @param payload - The request payload
   * @param requestId - The request ID for the response
   * @param registry - Stream registry for binding stream arguments
   * @param taskSender - Callback to send TaskMessage (Data/Close/Response)
   */
  dispatch(
    methodId: bigint,
    payload: Uint8Array,
    requestId: bigint,
    registry: ChannelRegistry,
    taskSender: TaskSender,
  ): Promise<void>;
}

/**
 * A live connection with completed Hello exchange.
 *
 * Generic over MessageTransport to support different transports
 * (CobsFramed for TCP, WsTransport for WebSocket).
 */
export class Connection<T extends MessageTransport = MessageTransport> {
  private io: T;
  private _role: Role;
  private _negotiated: Negotiated;
  private ourHello: Hello;
  private channelAllocator: ChannelIdAllocator;
  private channelRegistry: ChannelRegistry;
  private nextRequestId: bigint = 1n;

  constructor(io: T, role: Role, negotiated: Negotiated, ourHello: Hello) {
    this.io = io;
    this._role = role;
    this._negotiated = negotiated;
    this.ourHello = ourHello;
    this.channelAllocator = new ChannelIdAllocator(role);
    this.channelRegistry = new ChannelRegistry();
  }

  /** Get the underlying transport. */
  getIo(): T {
    return this.io;
  }

  /** Get the negotiated parameters. */
  negotiated(): Negotiated {
    return this._negotiated;
  }

  /** Get the connection role. */
  role(): Role {
    return this._role;
  }

  /**
   * Get the channel ID allocator.
   *
   * r[impl streaming.allocation.caller] - Caller allocates ALL channel IDs.
   */
  getChannelAllocator(): ChannelIdAllocator {
    return this.channelAllocator;
  }

  /**
   * Get the channel registry.
   */
  getChannelRegistry(): ChannelRegistry {
    return this.channelRegistry;
  }

  /**
   * Send a Goodbye message and return an error.
   *
   * r[impl message.goodbye.send] - Send Goodbye with rule ID before closing.
   * r[impl core.error.goodbye-reason] - Reason contains violated rule ID.
   */
  async goodbye(ruleId: string): Promise<ConnectionError> {
    try {
      await this.io.send(encodeGoodbye(ruleId));
    } catch {
      // Ignore send errors when closing
    }
    this.io.close();
    return ConnectionError.protocol(ruleId, "");
  }

  /**
   * Validate a channel ID according to protocol rules.
   *
   * Returns the rule ID if validation fails.
   */
  validateChannelId(channelId: bigint): string | null {
    // r[impl streaming.id.zero-reserved] - Channel ID 0 is reserved.
    if (channelId === 0n) {
      return "streaming.id.zero-reserved";
    }

    // r[impl streaming.unknown] - Unknown channel IDs are connection errors.
    if (!this.channelRegistry.contains(channelId)) {
      return "streaming.unknown";
    }

    return null;
  }

  /**
   * Send all pending outgoing channel messages.
   *
   * Drains the outgoing channels and sends Data/Close messages
   * to the peer. Call this periodically or after processing requests.
   *
   * r[impl streaming.data] - Send Data messages for outgoing channels.
   * r[impl streaming.close] - Send Close messages when channels end.
   */
  async flushOutgoing(): Promise<void> {
    while (true) {
      const poll = await this.channelRegistry.waitOutgoing();
      if (poll.kind === "pending" || poll.kind === "done") {
        break;
      }
      if (poll.kind === "data") {
        await this.io.send(encodeData(poll.channelId, poll.payload));
      } else if (poll.kind === "close") {
        await this.io.send(encodeClose(poll.channelId));
      }
    }
  }

  /**
   * Validate payload size against negotiated limit.
   *
   * r[impl flow.unary.payload-limit] - Payloads bounded by max_payload_size.
   * r[impl message.hello.negotiation] - Effective limit is min of both peers.
   */
  validatePayloadSize(size: number): string | null {
    if (size > this._negotiated.maxPayloadSize) {
      return "flow.unary.payload-limit";
    }
    return null;
  }

  /**
   * Make a unary RPC call.
   *
   * r[impl core.call] - Caller sends Request, callee responds with Response.
   * r[impl unary.complete] - Request gets exactly one Response.
   *
   * @param methodId - The method ID to call
   * @param payload - The request payload (already encoded)
   * @param timeoutMs - Timeout in milliseconds (default: 30000)
   * @returns The response payload
   */
  async call(
    methodId: bigint,
    payload: Uint8Array,
    timeoutMs: number = 30000,
  ): Promise<Uint8Array> {
    const requestId = this.nextRequestId++;

    // Send request
    await this.io.send(encodeRequest(requestId, methodId, payload));

    // Flush any pending outgoing stream data (for client-to-server streaming)
    // r[impl streaming.data] - Send queued Data/Close messages after Request.
    await this.flushOutgoing();

    // Wait for response
    while (true) {
      const data = await this.io.recvTimeout(timeoutMs);
      if (!data) {
        throw ConnectionError.io("timeout waiting for response");
      }

      // Parse message discriminant
      let offset = 0;
      const d0 = decodeVarintNumber(data, offset);
      const msgDisc = d0.value;
      offset = d0.next;

      if (msgDisc === MSG_GOODBYE) {
        throw ConnectionError.closed();
      }

      // Handle streaming messages while waiting for response
      if (msgDisc === MSG_DATA) {
        // Data { stream_id, payload }
        const sid = decodeVarint(data, offset);
        offset = sid.next;
        const pLen = decodeVarintNumber(data, offset);
        offset = pLen.next;
        const dataPayload = data.subarray(offset, offset + pLen.value);
        // Route to registered stream
        try {
          this.channelRegistry.routeData(sid.value, dataPayload);
        } catch {
          // Ignore stream errors during call - connection still valid
        }
        continue;
      }

      if (msgDisc === MSG_CLOSE) {
        // Close { stream_id }
        const sid = decodeVarint(data, offset);
        if (this.channelRegistry.contains(sid.value)) {
          this.channelRegistry.close(sid.value);
        }
        continue;
      }

      if (msgDisc === MSG_CREDIT) {
        // Credit { stream_id, amount } - flow control, currently ignored
        // TODO: Implement flow control tracking
        continue;
      }

      if (msgDisc !== MSG_RESPONSE) {
        // Ignore other messages (Hello after handshake, Reset, etc.)
        continue;
      }

      // Parse Response { request_id, metadata, payload }
      const reqIdResult = decodeVarint(data, offset);
      offset = reqIdResult.next;

      // Skip metadata: Vec<(String, MetadataValue)>
      const mdLen = decodeVarintNumber(data, offset);
      offset = mdLen.next;
      for (let i = 0; i < mdLen.value; i++) {
        // key string
        const kLen = decodeVarintNumber(data, offset);
        offset = kLen.next + kLen.value;
        // value enum
        const vDisc = decodeVarintNumber(data, offset);
        offset = vDisc.next;
        if (vDisc.value === 0) {
          // String
          const sLen = decodeVarintNumber(data, offset);
          offset = sLen.next + sLen.value;
        } else if (vDisc.value === 1) {
          // Bytes
          const bLen = decodeVarintNumber(data, offset);
          offset = bLen.next + bLen.value;
        } else if (vDisc.value === 2) {
          // U64
          const u = decodeVarint(data, offset);
          offset = u.next;
        }
      }

      // Payload
      const pLen = decodeVarintNumber(data, offset);
      offset = pLen.next;
      const responsePayload = data.subarray(offset, offset + pLen.value);

      // Check if this response is for our request
      if (reqIdResult.value === requestId) {
        return responsePayload;
      }
      // Otherwise continue waiting (might be response to different pipelined request)
    }
  }

  /**
   * Run the message loop with a streaming-aware dispatcher.
   *
   * This is the main event loop that:
   * - Receives messages from the peer
   * - Validates them according to protocol rules
   * - Dispatches requests to the service with stream binding
   * - Collects TaskMessages and sends them in order (Data/Close before Response)
   *
   * r[impl unary.pipelining.allowed] - Handle requests as they arrive.
   * r[impl unary.pipelining.independence] - Each request handled independently.
   */
  async runStreaming(dispatcher: StreamingDispatcher): Promise<void> {
    // Queue for task messages from handlers - handlers push, we flush
    const taskQueue: TaskMessage[] = [];

    // Track in-flight handler promises
    const inFlightHandlers: Set<Promise<void>> = new Set();

    // Signal for when a handler produces output or completes (to wake up the event loop)
    let wakeupResolve: (() => void) | null = null;
    const signalWakeup = () => {
      if (wakeupResolve) {
        wakeupResolve();
        wakeupResolve = null;
      }
    };

    // Task sender that queues messages and signals wakeup
    const taskSender: TaskSender = (msg) => {
      taskQueue.push(msg);
      signalWakeup(); // Wake up the event loop to flush
    };

    // Helper to flush task queue to wire
    const flushTaskQueue = async () => {
      while (taskQueue.length > 0) {
        const msg = taskQueue.shift()!;
        switch (msg.kind) {
          case "data":
            await this.io.send(encodeData(msg.channelId, msg.payload));
            break;
          case "close":
            await this.io.send(encodeClose(msg.channelId));
            break;
          case "response":
            await this.io.send(encodeResponse(msg.requestId, msg.payload));
            break;
        }
      }
    };

    // Pending receive promise (reused across iterations)
    let pendingRecv: Promise<
      { kind: "message"; payload: Uint8Array | null } | { kind: "error"; error: unknown }
    > | null = null;

    while (true) {
      // Flush any pending task messages from handlers
      await flushTaskQueue();

      // Start receiving if we don't have a pending receive
      if (!pendingRecv) {
        pendingRecv = this.io
          .recvTimeout(30000)
          .then((payload) => ({ kind: "message" as const, payload }))
          .catch((error) => ({ kind: "error" as const, error }));
      }

      // Create a promise that resolves when a handler produces output or completes
      const wakeupPromise = new Promise<void>((resolve) => {
        wakeupResolve = resolve;
      });

      // Always race between recv and wakeup (for task queue flushing)
      let recvResult:
        | { kind: "message"; payload: Uint8Array | null }
        | { kind: "error"; error: unknown }
        | null = null;

      const raceResult = await Promise.race([
        pendingRecv.then((r) => ({ source: "recv" as const, result: r })),
        wakeupPromise.then(() => ({ source: "wakeup" as const })),
      ]);

      if (raceResult.source === "wakeup") {
        // Wakeup signal (handler output or completion) - loop again to flush and continue
        continue;
      }
      recvResult = raceResult.result;

      // Clear pending recv since we consumed it
      pendingRecv = null;

      if (recvResult.kind === "error") {
        const raw = this.io.lastDecoded;
        if (raw.length >= 2 && raw[0] === 0x00 && raw[1] !== 0x00) {
          throw await this.goodbye("message.hello.unknown-version");
        }
        throw ConnectionError.io(String(recvResult.error));
      }

      const payload = recvResult.payload;
      if (!payload) {
        // Connection closed - wait for all in-flight handlers to complete
        await Promise.all(inFlightHandlers);
        await flushTaskQueue();
        return;
      }

      try {
        const handlerPromise = this.handleStreamingMessage(payload, dispatcher, taskSender);

        // If this returned a handler promise, track it
        if (handlerPromise) {
          inFlightHandlers.add(handlerPromise);
          handlerPromise.finally(() => {
            inFlightHandlers.delete(handlerPromise);
            signalWakeup();
          });
        }
      } catch (e) {
        if (e instanceof ConnectionError) {
          // For protocol errors, send Goodbye before closing
          if (e.kind === "protocol" && e.ruleId) {
            throw await this.goodbye(e.ruleId);
          }
          throw e;
        }
        throw await this.goodbye("message.decode-error");
      }
    }
  }

  /**
   * Handle a message in streaming mode.
   *
   * Returns a Promise for Request messages (the handler running concurrently),
   * or undefined for other message types that are processed synchronously.
   */
  private handleStreamingMessage(
    payload: Uint8Array,
    dispatcher: StreamingDispatcher,
    taskSender: TaskSender,
  ): Promise<void> | undefined {
    let offset = 0;
    const d0 = decodeVarintNumber(payload, offset);
    const msgDisc = d0.value;
    offset = d0.next;

    if (msgDisc === MSG_HELLO) {
      return undefined; // Duplicate Hello after exchange - ignore
    }

    if (msgDisc === MSG_GOODBYE) {
      throw ConnectionError.closed();
    }

    if (msgDisc === MSG_REQUEST) {
      // Request { request_id, method_id, metadata, payload }
      let tmp = decodeVarint(payload, offset);
      const requestId = tmp.value;
      offset = tmp.next;

      tmp = decodeVarint(payload, offset);
      const methodId = tmp.value;
      offset = tmp.next;

      // Skip metadata
      const mdLen = decodeVarintNumber(payload, offset);
      offset = mdLen.next;
      for (let i = 0; i < mdLen.value; i++) {
        const kLen = decodeVarintNumber(payload, offset);
        offset = kLen.next + kLen.value;
        const vDisc = decodeVarintNumber(payload, offset);
        offset = vDisc.next;
        if (vDisc.value === 0) {
          const sLen = decodeVarintNumber(payload, offset);
          offset = sLen.next + sLen.value;
        } else if (vDisc.value === 1) {
          const bLen = decodeVarintNumber(payload, offset);
          offset = bLen.next + bLen.value;
        } else if (vDisc.value === 2) {
          const u = decodeVarint(payload, offset);
          offset = u.next;
        } else {
          throw new Error("unknown MetadataValue");
        }
      }

      // payload: bytes
      const pLen = decodeVarintNumber(payload, offset);
      offset = pLen.next;

      // r[impl flow.unary.payload-limit] - Validate payload size
      const payloadViolation = this.validatePayloadSize(pLen.value);
      if (payloadViolation) {
        throw ConnectionError.protocol(payloadViolation, "payload exceeds max size");
      }

      const start = offset;
      const end = start + pLen.value;
      if (end > payload.length) throw new Error("request payload overrun");
      const payloadBytes = payload.subarray(start, end);

      // Dispatch with streaming support - return the promise, don't await it!
      // This allows the handler to run concurrently while we continue receiving messages.
      return dispatcher.dispatch(
        methodId,
        payloadBytes,
        requestId,
        this.channelRegistry,
        taskSender,
      );
    }

    // Handle other message types (Data, Close, etc.) synchronously
    if (msgDisc === MSG_RESPONSE) {
      return undefined;
    }

    if (msgDisc === MSG_DATA) {
      const sid = decodeVarint(payload, offset);
      offset = sid.next;
      if (sid.value === 0n) {
        // Can't send goodbye synchronously - throw protocol error
        throw ConnectionError.protocol("streaming.id.zero-reserved", "stream ID 0 is reserved");
      }
      const pLen = decodeVarintNumber(payload, offset);
      offset = pLen.next;
      const dataPayload = payload.subarray(offset, offset + pLen.value);
      try {
        this.channelRegistry.routeData(sid.value, dataPayload);
      } catch (e) {
        if (e instanceof ChannelError) {
          if (e.kind === "unknown") {
            throw ConnectionError.protocol("streaming.unknown", "unknown stream ID");
          }
          if (e.kind === "dataAfterClose") {
            throw ConnectionError.protocol("streaming.data-after-close", "data after close");
          }
        }
        throw e;
      }
      return undefined;
    }

    if (msgDisc === MSG_CLOSE) {
      const sid = decodeVarint(payload, offset);
      if (sid.value === 0n) {
        throw ConnectionError.protocol("streaming.id.zero-reserved", "stream ID 0 is reserved");
      }
      if (!this.channelRegistry.contains(sid.value)) {
        throw ConnectionError.protocol("streaming.unknown", "unknown stream ID");
      }
      this.channelRegistry.close(sid.value);
      return undefined;
    }

    if (msgDisc === MSG_RESET) {
      const sid = decodeVarint(payload, offset);
      if (sid.value === 0n) {
        throw ConnectionError.protocol("streaming.id.zero-reserved", "stream ID 0 is reserved");
      }
      if (!this.channelRegistry.contains(sid.value)) {
        throw ConnectionError.protocol("streaming.unknown", "unknown stream ID");
      }
      this.channelRegistry.close(sid.value);
      return undefined;
    }

    if (msgDisc === MSG_CREDIT) {
      const sid = decodeVarint(payload, offset);
      if (sid.value === 0n) {
        throw ConnectionError.protocol("streaming.id.zero-reserved", "stream ID 0 is reserved");
      }
      if (!this.channelRegistry.contains(sid.value)) {
        throw ConnectionError.protocol("streaming.unknown", "unknown stream ID");
      }
      return undefined;
    }

    return undefined; // Unknown message type - ignore
  }

  /**
   * Run the message loop with a dispatcher.
   *
   * This is the main event loop that:
   * - Receives messages from the peer
   * - Validates them according to protocol rules
   * - Dispatches requests to the service
   * - Sends responses back
   *
   * r[impl unary.pipelining.allowed] - Handle requests as they arrive.
   * r[impl unary.pipelining.independence] - Each request handled independently.
   */
  async run(dispatcher: ServiceDispatcher): Promise<void> {
    while (true) {
      let payload: Uint8Array | null;
      try {
        payload = await this.io.recvTimeout(30000);
      } catch (e) {
        // r[impl message.hello.unknown-version] - Reject unknown Hello versions.
        // Check for unknown Hello variant: [Message::Hello=0][Hello::unknown=1+]
        const raw = this.io.lastDecoded;
        if (raw.length >= 2 && raw[0] === 0x00 && raw[1] !== 0x00) {
          throw await this.goodbye("message.hello.unknown-version");
        }
        throw ConnectionError.io(String(e));
      }

      if (!payload) {
        return; // Connection closed or timeout
      }

      try {
        await this.handleMessage(payload, dispatcher);
      } catch (e) {
        if (e instanceof ConnectionError) throw e;
        // r[impl message.decode-error] - send goodbye on decode failure
        throw await this.goodbye("message.decode-error");
      }
    }
  }

  private async handleMessage(payload: Uint8Array, dispatcher: ServiceDispatcher): Promise<void> {
    let offset = 0;
    const d0 = decodeVarintNumber(payload, offset);
    const msgDisc = d0.value;
    offset = d0.next;

    if (msgDisc === MSG_HELLO) {
      // Duplicate Hello after exchange - ignore
      return;
    }

    if (msgDisc === MSG_GOODBYE) {
      // Peer sent Goodbye, connection closing
      throw ConnectionError.closed();
    }

    if (msgDisc === MSG_REQUEST) {
      // Request { request_id, method_id, metadata, payload }
      let tmp = decodeVarint(payload, offset);
      const requestId = tmp.value;
      offset = tmp.next;

      tmp = decodeVarint(payload, offset);
      const methodId = tmp.value;
      offset = tmp.next;

      // Skip metadata: Vec<(String, MetadataValue)>
      const mdLen = decodeVarintNumber(payload, offset);
      offset = mdLen.next;
      for (let i = 0; i < mdLen.value; i++) {
        // key string
        const kLen = decodeVarintNumber(payload, offset);
        offset = kLen.next + kLen.value;
        // value enum
        const vDisc = decodeVarintNumber(payload, offset);
        offset = vDisc.next;
        if (vDisc.value === 0) {
          // String
          const sLen = decodeVarintNumber(payload, offset);
          offset = sLen.next + sLen.value;
        } else if (vDisc.value === 1) {
          // Bytes
          const bLen = decodeVarintNumber(payload, offset);
          offset = bLen.next + bLen.value;
        } else if (vDisc.value === 2) {
          // U64
          const u = decodeVarint(payload, offset);
          offset = u.next;
        } else {
          throw new Error("unknown MetadataValue");
        }
      }

      // payload: bytes
      const pLen = decodeVarintNumber(payload, offset);
      offset = pLen.next;

      // r[impl flow.unary.payload-limit] - enforce negotiated max payload size
      const payloadViolation = this.validatePayloadSize(pLen.value);
      if (payloadViolation) {
        throw await this.goodbye(payloadViolation);
      }

      const start = offset;
      const end = start + pLen.value;
      if (end > payload.length) throw new Error("request payload overrun");
      const payloadBytes = payload.subarray(start, end);

      // Dispatch to service
      const responsePayload = await dispatcher.dispatchUnary(methodId, payloadBytes);

      // r[impl core.call] - Callee sends Response for caller's Request.
      // r[impl core.call.request-id] - Response has same request_id.
      // r[impl unary.complete] - Send Response with matching request_id.
      // r[impl unary.lifecycle.single-response] - Exactly one Response per Request.
      await this.io.send(encodeResponse(requestId, responsePayload));

      // Flush any outgoing stream data that handlers may have queued
      await this.flushOutgoing();
      return;
    }

    if (msgDisc === MSG_RESPONSE) {
      // Server doesn't expect Response in basic mode - skip
      // Skip over the fields to not break parsing
      return;
    }

    if (msgDisc === MSG_DATA) {
      // Data { stream_id, payload }
      const sid = decodeVarint(payload, offset);
      offset = sid.next;

      // r[impl streaming.id.zero-reserved] - Stream ID 0 is reserved.
      if (sid.value === 0n) {
        throw await this.goodbye("streaming.id.zero-reserved");
      }

      // Decode payload
      const pLen = decodeVarintNumber(payload, offset);
      offset = pLen.next;
      const dataPayload = payload.subarray(offset, offset + pLen.value);

      // r[impl streaming.data] - Route Data to registered stream.
      try {
        this.channelRegistry.routeData(sid.value, dataPayload);
      } catch (e) {
        if (e instanceof ChannelError) {
          if (e.kind === "unknown") {
            // r[impl streaming.unknown] - Unknown stream ID.
            throw await this.goodbye("streaming.unknown");
          }
          if (e.kind === "dataAfterClose") {
            // r[impl streaming.data-after-close] - Data after Close is error.
            throw await this.goodbye("streaming.data-after-close");
          }
        }
        throw e;
      }
      return;
    }

    if (msgDisc === MSG_CLOSE) {
      // Close { stream_id }
      const sid = decodeVarint(payload, offset);

      // r[impl streaming.id.zero-reserved] - Stream ID 0 is reserved.
      if (sid.value === 0n) {
        throw await this.goodbye("streaming.id.zero-reserved");
      }

      // r[impl streaming.close] - Close the stream.
      if (!this.channelRegistry.contains(sid.value)) {
        throw await this.goodbye("streaming.unknown");
      }
      this.channelRegistry.close(sid.value);
      return;
    }

    if (msgDisc === MSG_RESET) {
      // Reset { stream_id }
      const sid = decodeVarint(payload, offset);

      // r[impl streaming.id.zero-reserved] - Stream ID 0 is reserved.
      if (sid.value === 0n) {
        throw await this.goodbye("streaming.id.zero-reserved");
      }

      // r[impl streaming.reset] - Forcefully terminate stream.
      // For now, treat same as Close.
      // TODO: Signal error to Pull<T> instead of clean close.
      if (!this.channelRegistry.contains(sid.value)) {
        throw await this.goodbye("streaming.unknown");
      }
      this.channelRegistry.close(sid.value);
      return;
    }

    if (msgDisc === MSG_CREDIT) {
      // Credit { stream_id, amount }
      const sid = decodeVarint(payload, offset);

      // r[impl streaming.id.zero-reserved] - Stream ID 0 is reserved.
      if (sid.value === 0n) {
        throw await this.goodbye("streaming.id.zero-reserved");
      }

      // TODO: Implement flow control.
      // For now, validate stream exists but ignore credit.
      if (!this.channelRegistry.contains(sid.value)) {
        throw await this.goodbye("streaming.unknown");
      }
      return;
    }

    // Unknown message type - ignore
  }
}

/**
 * Perform Hello exchange as the acceptor (server).
 *
 * r[impl message.hello.timing] - Send Hello immediately after connection.
 * r[impl message.hello.ordering] - Hello sent before any other message.
 */
export async function helloExchangeAcceptor<T extends MessageTransport>(
  io: T,
  ourHello: Hello,
): Promise<Connection<T>> {
  // Send our Hello immediately
  await io.send(encodeHello(ourHello));

  // Wait for peer Hello
  const peerHello = await waitForPeerHello(io, ourHello);

  // r[impl message.hello.negotiation] - Effective limit is min of both peers.
  const negotiated: Negotiated = {
    maxPayloadSize: Math.min(ourHello.maxPayloadSize, peerHello.maxPayloadSize),
    initialCredit: Math.min(ourHello.initialStreamCredit, peerHello.initialStreamCredit),
  };

  return new Connection(io, Role.Acceptor, negotiated, ourHello);
}

/**
 * Perform Hello exchange as the initiator (client).
 *
 * r[impl message.hello.timing] - Send Hello immediately after connection.
 * r[impl message.hello.ordering] - Hello sent before any other message.
 */
export async function helloExchangeInitiator<T extends MessageTransport>(
  io: T,
  ourHello: Hello,
): Promise<Connection<T>> {
  // Send our Hello immediately
  await io.send(encodeHello(ourHello));

  // Wait for peer Hello
  const peerHello = await waitForPeerHello(io, ourHello);

  const negotiated: Negotiated = {
    maxPayloadSize: Math.min(ourHello.maxPayloadSize, peerHello.maxPayloadSize),
    initialCredit: Math.min(ourHello.initialStreamCredit, peerHello.initialStreamCredit),
  };

  return new Connection(io, Role.Initiator, negotiated, ourHello);
}

async function waitForPeerHello<T extends MessageTransport>(
  io: T,
  _ourHello: Hello,
): Promise<Hello> {
  while (true) {
    let payload: Uint8Array | null;
    try {
      payload = await io.recvTimeout(5000);
    } catch {
      // r[impl message.hello.unknown-version] - Reject unknown Hello versions.
      const raw = io.lastDecoded;
      if (raw.length >= 2 && raw[0] === 0x00 && raw[1] !== 0x00) {
        await io.send(encodeGoodbye("message.hello.unknown-version"));
        io.close();
        throw ConnectionError.protocol("message.hello.unknown-version", "unknown Hello variant");
      }
      throw ConnectionError.io("failed to receive peer Hello");
    }

    if (!payload) {
      throw ConnectionError.closed();
    }

    // Parse message discriminant
    const d0 = decodeVarintNumber(payload, 0);
    const msgDisc = d0.value;
    let offset = d0.next;

    if (msgDisc === MSG_HELLO) {
      // Parse Hello
      const d1 = decodeVarintNumber(payload, offset);
      const helloVariant = d1.value;
      offset = d1.next;

      // r[impl message.hello.unknown-version] - reject unknown Hello versions
      if (helloVariant !== 0) {
        await io.send(encodeGoodbye("message.hello.unknown-version"));
        io.close();
        throw ConnectionError.protocol("message.hello.unknown-version", "unknown Hello variant");
      }

      const maxPayload = decodeVarintNumber(payload, offset);
      offset = maxPayload.next;
      const initialCredit = decodeVarintNumber(payload, offset);

      return {
        variant: 0,
        maxPayloadSize: maxPayload.value,
        initialStreamCredit: initialCredit.value,
      };
    }

    // Received non-Hello before Hello exchange completed
    await io.send(encodeGoodbye("message.hello.ordering"));
    io.close();
    throw ConnectionError.protocol(
      "message.hello.ordering",
      "received non-Hello before Hello exchange",
    );
  }
}

/** Default Hello message. */
export function defaultHello(): Hello {
  return {
    variant: 0,
    maxPayloadSize: 1024 * 1024,
    initialStreamCredit: 64 * 1024,
  };
}
