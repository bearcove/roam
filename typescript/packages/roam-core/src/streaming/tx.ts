// Tx stream handle - caller sends data to callee.

import { type StreamId, StreamError } from "./types.ts";
import { OutgoingSender } from "./registry.ts";
import { type TaskSender } from "./task.ts";

/**
 * Sender abstraction for Tx streams.
 *
 * Supports two modes:
 * - Client-side: uses OutgoingSender (buffered channel to drain task)
 * - Server-side: uses TaskSender (direct to connection driver)
 */
type TxSender =
  | { mode: "client"; sender: OutgoingSender }
  | { mode: "server"; channelId: StreamId; taskSender: TaskSender };

/**
 * Tx stream handle - caller sends data to callee.
 *
 * r[impl streaming.caller-pov] - From caller's perspective, Tx means "I send".
 * r[impl streaming.type] - Serializes as u64 stream ID on wire.
 * r[impl streaming.holder-semantics] - The holder sends on this stream.
 *
 * # Two modes of operation
 *
 * - **Client side**: Uses OutgoingSender which buffers to a drain task.
 *   Created via `createTypedTx()` or `Connection.createTx()`.
 * - **Server side**: Uses TaskSender to send Data/Close directly to driver.
 *   Created via `createServerTx()` in generated dispatch code.
 *
 * @template T - The type of values being sent (needs a serializer).
 */
export class Tx<T> {
  private closed = false;
  private _streamId: StreamId;
  private sender: TxSender;
  private serialize: (value: T) => Uint8Array;

  /** Create a client-side Tx with an OutgoingSender. */
  constructor(sender: OutgoingSender, serialize: (value: T) => Uint8Array);
  /** Create a server-side Tx with a TaskSender. */
  constructor(channelId: StreamId, taskSender: TaskSender, serialize: (value: T) => Uint8Array);
  constructor(
    senderOrChannelId: OutgoingSender | StreamId,
    serializeOrTaskSender: ((value: T) => Uint8Array) | TaskSender,
    maybeSerialize?: (value: T) => Uint8Array,
  ) {
    if (typeof senderOrChannelId === "bigint") {
      // Server-side constructor
      this._streamId = senderOrChannelId;
      this.sender = {
        mode: "server",
        channelId: senderOrChannelId,
        taskSender: serializeOrTaskSender as TaskSender,
      };
      this.serialize = maybeSerialize!;
    } else {
      // Client-side constructor
      this._streamId = senderOrChannelId.streamId;
      this.sender = { mode: "client", sender: senderOrChannelId };
      this.serialize = serializeOrTaskSender as (value: T) => Uint8Array;
    }
  }

  /** Get the stream ID. */
  get streamId(): StreamId {
    return this._streamId;
  }

  /**
   * Send a value on this stream.
   *
   * r[impl streaming.data] - Data messages carry serialized values.
   */
  send(value: T): void {
    if (this.closed) {
      throw StreamError.closed();
    }

    let bytes: Uint8Array;
    try {
      bytes = this.serialize(value);
    } catch (e) {
      throw StreamError.serialize(e);
    }

    if (this.sender.mode === "client") {
      if (!this.sender.sender.sendData(bytes)) {
        throw StreamError.closed();
      }
    } else {
      // Server-side: send directly via task channel
      this.sender.taskSender({
        kind: "data",
        channelId: this.sender.channelId,
        payload: bytes,
      });
    }
  }

  /**
   * Close this stream.
   *
   * r[impl streaming.lifecycle.caller-closes-pushes] - Caller sends Close when done.
   */
  close(): void {
    if (this.closed) return;
    this.closed = true;

    if (this.sender.mode === "client") {
      this.sender.sender.sendClose();
    } else {
      // Server-side: send Close via task channel
      this.sender.taskSender({
        kind: "close",
        channelId: this.sender.channelId,
      });
    }
  }
}

// Note: Symbol.dispose support for using-declarations would be nice but requires esnext target.
// For now, users should call close() explicitly or use try/finally.

/**
 * Create a Tx stream with a simple passthrough (for raw bytes).
 */
export function createRawTx(sender: OutgoingSender): Tx<Uint8Array> {
  return new Tx(sender, (v) => v);
}

/**
 * Create a Tx stream with a typed serializer (client-side).
 *
 * r[impl streaming.type] - Tx serializes as stream_id on wire.
 */
export function createTypedTx<T>(
  sender: OutgoingSender,
  serialize: (value: T) => Uint8Array,
): Tx<T> {
  return new Tx(sender, serialize);
}

/**
 * Create a server-side Tx stream that sends directly via the task channel.
 *
 * Used by generated dispatch code to hydrate Tx arguments.
 * When the handler calls tx.send(), Data messages go directly to the driver.
 * When the handler is done and calls tx.close(), a Close message is sent.
 *
 * @param channelId - The stream ID from the wire (allocated by caller)
 * @param taskSender - Callback to send TaskMessage to driver
 * @param serialize - Function to serialize values to bytes
 */
export function createServerTx<T>(
  channelId: StreamId,
  taskSender: TaskSender,
  serialize: (value: T) => Uint8Array,
): Tx<T> {
  return new Tx(channelId, taskSender, serialize);
}
