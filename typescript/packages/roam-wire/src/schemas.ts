// Roam wire protocol schemas for TypeScript.
//
// Source of truth: generated from rust/roam-types facet shapes.

import type { Schema, SchemaRegistry } from "@bearcove/roam-postcard";
export {
  ParitySchema,
  ConnectionSettingsSchema,
  MetadataValueSchema,
  MetadataEntrySchema,
  HelloSchema,
  HelloYourselfSchema,
  ProtocolErrorSchema,
  ConnectionOpenSchema,
  ConnectionAcceptSchema,
  ConnectionRejectSchema,
  ConnectionCloseSchema,
  RequestBodySchema,
  RequestMessageSchema,
  ChannelBodySchema,
  ChannelMessageSchema,
  MessagePayloadSchema,
  MessageSchema,
  wireSchemaRegistry,
} from "./schemas.generated.ts";

import {
  HelloSchema,
  MetadataValueSchema,
  MetadataEntrySchema,
  MessageSchema,
  wireSchemaRegistry,
} from "./schemas.generated.ts";

export function getHelloSchema(): Schema {
  return HelloSchema;
}

export function getMetadataValueSchema(): Schema {
  return MetadataValueSchema;
}

export function getMetadataEntrySchema(): Schema {
  return MetadataEntrySchema;
}

export function getMessageSchema(): Schema {
  return MessageSchema;
}

export { type Schema, type SchemaRegistry };
