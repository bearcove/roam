// Task message types for server-side streaming.
//
// All messages from spawned handler tasks go through a single channel to preserve ordering.
// This ensures Data/Close messages are sent before the Response.

import { type StreamId } from "./types.ts";

/**
 * Message from spawned handler tasks to the connection driver.
 *
 * All messages from tasks go through a single channel to preserve ordering.
 * This ensures Data/Close messages are sent before the Response.
 */
export type TaskMessage =
  | { kind: "data"; channelId: StreamId; payload: Uint8Array }
  | { kind: "close"; channelId: StreamId }
  | { kind: "response"; requestId: bigint; payload: Uint8Array };

/**
 * Callback for sending task messages to the connection driver.
 *
 * Used by server-side Tx handles to send Data/Close messages.
 */
export type TaskSender = (msg: TaskMessage) => void;

/**
 * Context for server-side stream dispatch.
 *
 * Provides the task sender and stream registry access needed
 * for handlers to work with Tx/Rx streams.
 */
export interface StreamContext {
  /**
   * Send a task message (Data, Close, or Response) to the connection driver.
   */
  sendTask: TaskSender;

  /**
   * Register an incoming stream and get a receiver for it.
   * Used for Rx<T> arguments where the server receives data from the client.
   */
  registerIncoming(channelId: StreamId): AsyncIterable<Uint8Array>;

  /**
   * Create a sender for an outgoing stream.
   * Used for Tx<T> arguments where the server sends data to the client.
   */
  createOutgoingSender(channelId: StreamId): (payload: Uint8Array) => void;
}
