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
  type MethodSchema,
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
  type BindingSerializers,
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

// Type definitions for method handlers
export type MethodHandler<H> = (handler: H, payload: Uint8Array) => Promise<Uint8Array>;

// Generic RPC dispatcher
export class RpcDispatcher<H> {
  private methodHandlers: Map<bigint, MethodHandler<H>>;

  constructor(methodHandlers: Map<bigint, MethodHandler<H>>) {
    this.methodHandlers = methodHandlers;
  }

  async dispatch(handler: H, methodId: bigint, payload: Uint8Array): Promise<Uint8Array> {
    const methodHandler = this.methodHandlers.get(methodId);
    if (!methodHandler) {
      // r[impl call.error.unknown-method]
      // Err(RoamError::UnknownMethod) = [0x01=Err, 0x01=UnknownMethod]
      return new Uint8Array([0x01, 0x01]);
    }

    // Method handlers are responsible for their own error handling:
    // - Decode errors return InvalidPayload (per r[impl call.error.invalid-payload])
    // - Fallible methods encode errors as RoamError::User(E)
    // - Infallible methods that throw indicate bugs and should propagate
    return await methodHandler(handler, payload);
  }
}

/** @deprecated Use RpcDispatcher instead */
export const UnaryDispatcher = RpcDispatcher;
