// TODO: move to roam-wire

import { encodeWithSchema } from "./schema_codec.ts";
import { RoamErrorPayloadSchema, rpcWireSchemaRegistry } from "./result.ts";

export const ROAM_ERROR = {
  USER: 0,
  UNKNOWN_METHOD: 1,
  INVALID_PAYLOAD: 2,
  CANCELLED: 3,
} as const;

export function encodeUnknownMethod(): Uint8Array {
  return encodeWithSchema(
    { tag: "UnknownMethod" },
    RoamErrorPayloadSchema,
    rpcWireSchemaRegistry,
  );
}

export function encodeInvalidPayload(): Uint8Array {
  return encodeWithSchema(
    { tag: "InvalidPayload" },
    RoamErrorPayloadSchema,
    rpcWireSchemaRegistry,
  );
}
