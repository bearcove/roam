import type { EnumSchema, Schema, SchemaRegistry } from "./schema.ts";
import { encodeWithSchema, decodeWithSchema } from "./schema_codec.ts";

export type RoamErrorPayload =
  | { tag: "User"; value: Uint8Array }
  | { tag: "UnknownMethod" }
  | { tag: "InvalidPayload" }
  | { tag: "Cancelled" };

export type RpcWireResult =
  | { tag: "Ok"; value: Uint8Array }
  | { tag: "Err"; value: RoamErrorPayload };

export const RoamErrorPayloadSchema: EnumSchema = {
  kind: "enum",
  variants: [
    { name: "User", discriminant: 0, fields: { kind: "bytes" } },
    { name: "UnknownMethod", discriminant: 1, fields: null },
    { name: "InvalidPayload", discriminant: 2, fields: null },
    { name: "Cancelled", discriminant: 3, fields: null },
  ],
};

export const RpcWireResultSchema: EnumSchema = {
  kind: "enum",
  variants: [
    { name: "Ok", discriminant: 0, fields: { kind: "bytes" } },
    { name: "Err", discriminant: 1, fields: { kind: "ref", name: "RoamErrorPayload" } },
  ],
};

export const rpcWireSchemaRegistry: SchemaRegistry = new Map<string, Schema>([
  ["RoamErrorPayload", RoamErrorPayloadSchema],
  ["RpcWireResult", RpcWireResultSchema],
]);

export function encodeResultOk(okBytes: Uint8Array): Uint8Array {
  return encodeWithSchema({ tag: "Ok", value: okBytes }, RpcWireResultSchema, rpcWireSchemaRegistry);
}

export function encodeResultErr(errBytes: Uint8Array): Uint8Array {
  const err = decodeWithSchema(
    errBytes,
    0,
    RoamErrorPayloadSchema,
    rpcWireSchemaRegistry,
  ) as { value: RoamErrorPayload; next: number };
  return encodeWithSchema({ tag: "Err", value: err.value }, RpcWireResultSchema, rpcWireSchemaRegistry);
}

export function decodeRpcWireResult(
  buf: Uint8Array,
  offset = 0,
): { value: RpcWireResult; next: number } {
  return decodeWithSchema(
    buf,
    offset,
    RpcWireResultSchema,
    rpcWireSchemaRegistry,
  ) as { value: RpcWireResult; next: number };
}
