// Wire codec wrappers for encoding/decoding Roam protocol messages.

import { encodeWithSchema, decodeWithSchema, type DecodeResult } from "@bearcove/roam-postcard";

import type { Hello, MetadataValue, MetadataEntry, Message } from "./types.ts";
import {
  HelloSchema,
  MetadataValueSchema,
  MetadataEntrySchema,
  MessageSchema,
  wireSchemaRegistry,
} from "./schemas.ts";

export function encodeHello(hello: Hello): Uint8Array {
  return encodeWithSchema(hello, HelloSchema, wireSchemaRegistry);
}

export function decodeHello(buf: Uint8Array, offset = 0): DecodeResult<Hello> {
  return decodeWithSchema(buf, offset, HelloSchema, wireSchemaRegistry) as DecodeResult<Hello>;
}

export function encodeMetadataValue(value: MetadataValue): Uint8Array {
  return encodeWithSchema(value, MetadataValueSchema, wireSchemaRegistry);
}

export function decodeMetadataValue(buf: Uint8Array, offset = 0): DecodeResult<MetadataValue> {
  return decodeWithSchema(
    buf,
    offset,
    MetadataValueSchema,
    wireSchemaRegistry,
  ) as DecodeResult<MetadataValue>;
}

export function encodeMetadataEntry(entry: MetadataEntry): Uint8Array {
  return encodeWithSchema(entry, MetadataEntrySchema, wireSchemaRegistry);
}

export function decodeMetadataEntry(buf: Uint8Array, offset = 0): DecodeResult<MetadataEntry> {
  return decodeWithSchema(
    buf,
    offset,
    MetadataEntrySchema,
    wireSchemaRegistry,
  ) as DecodeResult<MetadataEntry>;
}

export function encodeMessage(message: Message): Uint8Array {
  return encodeWithSchema(message, MessageSchema, wireSchemaRegistry);
}

export function decodeMessage(buf: Uint8Array, offset = 0): DecodeResult<Message> {
  return decodeWithSchema(buf, offset, MessageSchema, wireSchemaRegistry) as DecodeResult<Message>;
}

export function decodeMessages(buf: Uint8Array): Message[] {
  const messages: Message[] = [];
  let offset = 0;
  while (offset < buf.length) {
    const result = decodeMessage(buf, offset);
    messages.push(result.value);
    offset = result.next;
  }
  return messages;
}

export function encodeMessages(messages: Message[]): Uint8Array {
  const parts: Uint8Array[] = messages.map(encodeMessage);
  const totalLength = parts.reduce((sum, p) => sum + p.length, 0);
  const result = new Uint8Array(totalLength);
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}
