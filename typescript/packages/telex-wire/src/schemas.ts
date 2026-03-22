// Telex wire protocol schemas for TypeScript.
//
// Source of truth: generated from rust/telex-types facet shapes.

export type {
  Schema,
  SchemaRegistry,
  TypeRef,
} from "@bearcove/telex-postcard";
export {
  messageSchemasCbor,
  messageSchemaRegistry,
  messageRootRef,
} from "./wire.generated.ts";
