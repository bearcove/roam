// Channeling module exports

export { type ChannelId, Role, ChannelError } from "./types.ts";
export { ChannelIdAllocator } from "./allocator.ts";
export {
  createChannel,
  createChannelPair,
  ChannelSender,
  ChannelReceiver,
  type Channel,
} from "./channel.ts";
export {
  ChannelRegistry,
  OutgoingSender,
  type OutgoingMessage,
  type OutgoingPoll,
} from "./registry.ts";
export { Tx, createServerTx } from "./tx.ts";
export { Rx, createServerRx } from "./rx.ts";
export { channel } from "./pair.ts";
export { type TaskMessage, type TaskSender, type ChannelContext } from "./task.ts";
