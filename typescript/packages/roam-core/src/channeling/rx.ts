// Rx channel handle - caller receives data from callee.

import { type ChannelId, ChannelError } from "./types.ts";
import { ChannelReceiver } from "./channel.ts";

/**
 * Receiver abstraction for Rx channels.
 *
 * Both client and server side use the same ChannelReceiver,
 * but the channel is set up differently:
 * - Client side: Created via Connection.createRx(), data routed from incoming messages
 * - Server side: Created in dispatch, channel registered for incoming Data routing
 */

/**
 * Rx channel handle - caller receives data from callee.
 *
 * r[impl streaming.caller-pov] - From caller's perspective, Rx means "I receive".
 * r[impl streaming.type] - Serializes as u64 channel ID on wire.
 * r[impl streaming.holder-semantics] - The holder receives from this channel.
 *
 * @template T - The type of values being received (needs a deserializer).
 */
export class Rx<T> {
  constructor(
    private _channelId: ChannelId,
    private receiver: ChannelReceiver<Uint8Array>,
    private deserialize: (bytes: Uint8Array) => T,
  ) {}

  /** Get the channel ID. */
  get channelId(): ChannelId {
    return this._channelId;
  }

  /**
   * Receive the next value from this channel.
   *
   * Returns the value, or null when the channel is closed.
   *
   * r[impl streaming.data] - Deserialize Data message payloads.
   */
  async recv(): Promise<T | null> {
    const bytes = await this.receiver.recv();
    if (bytes === null) {
      return null; // Channel closed
    }

    try {
      return this.deserialize(bytes);
    } catch (e) {
      throw ChannelError.deserialize(e);
    }
  }

  /**
   * Iterate over all values in the channel.
   *
   * This is an async iterator that yields values until the channel closes.
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
 * Create an Rx channel with a simple passthrough (for raw bytes).
 */
export function createRawRx(
  channelId: ChannelId,
  receiver: ChannelReceiver<Uint8Array>,
): Rx<Uint8Array> {
  return new Rx(channelId, receiver, (v) => v);
}

/**
 * Create an Rx channel with a typed deserializer.
 *
 * r[impl streaming.type] - Rx serializes as channel_id on wire.
 */
export function createTypedRx<T>(
  channelId: ChannelId,
  receiver: ChannelReceiver<Uint8Array>,
  deserialize: (bytes: Uint8Array) => T,
): Rx<T> {
  return new Rx(channelId, receiver, deserialize);
}

/**
 * Create a server-side Rx channel.
 *
 * Used by generated dispatch code to hydrate Rx arguments.
 * The channel is registered with the channel registry for Data routing.
 *
 * @param channelId - The channel ID from the wire (allocated by caller)
 * @param receiver - Channel receiver for incoming Data payloads
 * @param deserialize - Function to deserialize bytes to values
 */
export function createServerRx<T>(
  channelId: ChannelId,
  receiver: ChannelReceiver<Uint8Array>,
  deserialize: (bytes: Uint8Array) => T,
): Rx<T> {
  return new Rx(channelId, receiver, deserialize);
}
