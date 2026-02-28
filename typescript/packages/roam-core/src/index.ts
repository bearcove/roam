// @bearcove/roam-runtime - TypeScript runtime for roam RPC
// This package provides the core primitives and dispatcher for roam services.

// RPC error types (for client-side error handling)
export {
  RpcError,
  RpcErrorCode,
  decodeUserError,
} from "@bearcove/roam-wire";

// Wire types, schemas, and codec
export type {
  Hello,
  MetadataValue,
  MetadataValueString,
  MetadataValueBytes,
  MetadataValueU64,
  MetadataEntry,
  Message,
  MessageHello,
  MessageConnect,
  MessageAccept,
  MessageReject,
  MessageGoodbye,
  MessageRequest,
  MessageResponse,
  MessageCancel,
  MessageData,
  MessageClose,
  MessageReset,
  MessageCredit,
} from "@bearcove/roam-wire";

export {
  // Metadata flags
  MetadataFlags,
  // Wire factory functions
  metadataString,
  metadataBytes,
  metadataU64,
  messageHello,
  messageConnect,
  messageAccept,
  messageReject,
  messageGoodbye,
  messageRequest,
  messageResponse,
  messageCancel,
  messageData,
  messageClose,
  messageReset,
  messageCredit,
  // Wire codec
  encodeMessage,
  decodeMessage,
} from "@bearcove/roam-wire";

// Channel types
export {
  type ChannelId,
  Role,
  ChannelError,
  ChannelIdAllocator,
  ChannelRegistry,
  OutgoingSender,
  ChannelReceiver,
  Tx,
  Rx,
  channel,
  createServerTx,
  createServerRx,
  type OutgoingMessage,
  type OutgoingPoll,
  type TaskMessage,
  type TaskSender,
  type ChannelContext,
  // Descriptor types
  type MethodDescriptor,
  type ServiceDescriptor,
  type RoamCall,
  // Schema types and binding
  type PrimitiveKind,
  type TxSchema,
  type RxSchema,
  type VecSchema,
  type OptionSchema,
  type MapSchema,
  type StructSchema,
  type TupleSchema,
  type EnumVariant,
  type EnumSchema,
  type RefSchema,
  type Schema,
  type SchemaRegistry,
  // Schema helper functions
  resolveSchema,
  findVariantByDiscriminant,
  findVariantByName,
  getVariantDiscriminant,
  getVariantFieldSchemas,
  getVariantFieldNames,
  isNewtypeVariant,
  isRefSchema,
  bindChannels,
} from "./channeling/index.ts";

// Transport abstraction
export { type MessageTransport } from "./transport.ts";

// Connection and protocol handling
export {
  Connection,
  ConnectionCaller,
  ConnectionError,
  type Negotiated,
  type ServiceDispatcher,
  type ChannelingDispatcher,
  type HelloExchangeOptions,
  helloExchangeAcceptor,
  helloExchangeInitiator,
  defaultHello,
} from "./connection.ts";

// Client middleware types
export {
  Extensions,
  type ClientContext,
  type CallRequest,
  type CallOutcome,
  type RejectionCode,
  type Rejection,
  RejectionError,
  type ClientMiddleware,
} from "./middleware.ts";

// Caller abstraction
export { type CallerRequest, type Caller, MiddlewareCaller } from "./caller.ts";

// Call builder for fluent API
export { CallBuilder, withMeta, type CallExecutor } from "./call_builder.ts";

// Logging middleware
export { loggingMiddleware, type LoggingOptions, type ErrorDecoder } from "./logging.ts";

// Metadata conversion utilities
export {
  ClientMetadata,
  type ClientMetadataValue,
  clientMetadataToEntries,
  metadataEntriesToClientMetadata,
} from "./metadata.ts";
