// Rx stream handle - caller receives data from callee.

import { type StreamId, StreamError } from "./types.ts";
import { ChannelReceiver } from "./channel.ts";

/**
 * Rx stream handle - caller receives data from callee.
 *
 * r[impl streaming.caller-pov] - From caller's perspective, Rx means "I receive".
 * r[impl streaming.type] - Serializes as u64 stream ID on wire.
 * r[impl streaming.holder-semantics] - The holder receives from this stream.
 *
 * @template T - The type of values being received (needs a deserializer).
 */
export class Rx<T> {
  constructor(
    private _streamId: StreamId,
    private receiver: ChannelReceiver<Uint8Array>,
    private deserialize: (bytes: Uint8Array) => T,
  ) {}

  /** Get the stream ID. */
  get streamId(): StreamId {
    return this._streamId;
  }

  /**
   * Receive the next value from this stream.
   *
   * Returns the value, or null when the stream is closed.
   *
   * r[impl streaming.data] - Deserialize Data message payloads.
   */
  async recv(): Promise<T | null> {
    const bytes = await this.receiver.recv();
    if (bytes === null) {
      return null; // Stream closed
    }

    try {
      return this.deserialize(bytes);
    } catch (e) {
      throw StreamError.deserialize(e);
    }
  }

  /**
   * Iterate over all values in the stream.
   *
   * This is an async iterator that yields values until the stream closes.
   */
  async *[Symbol.asyncIterator](): AsyncIterator<T> {
    while (true) {
      const value = await this.recv();
      if (value === null) {
        return;
      }
      yield value;
    }
  }
}

/**
 * Create an Rx stream with a simple passthrough (for raw bytes).
 */
export function createRawRx(
  streamId: StreamId,
  receiver: ChannelReceiver<Uint8Array>,
): Rx<Uint8Array> {
  return new Rx(streamId, receiver, (v) => v);
}

/**
 * Create an Rx stream with a typed deserializer.
 *
 * r[impl streaming.type] - Rx serializes as stream_id on wire.
 */
export function createTypedRx<T>(
  streamId: StreamId,
  receiver: ChannelReceiver<Uint8Array>,
  deserialize: (bytes: Uint8Array) => T,
): Rx<T> {
  return new Rx(streamId, receiver, deserialize);
}
