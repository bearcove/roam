// Runtime channel binder.
//
// Walks argument structures using schemas to find and bind Tx/Rx channels.

import type { Schema } from "./schema.ts";
import type { ChannelIdAllocator } from "./allocator.ts";
import type { ChannelRegistry } from "./registry.ts";
import { Tx } from "./tx.ts";
import { Rx } from "./rx.ts";

/**
 * Serializers for binding channels.
 *
 * When a channel is bound, we need to know how to serialize (for Tx)
 * or deserialize (for Rx) the element type. These are provided by
 * the generated code.
 */
export interface BindingSerializers {
  /** Get a serializer for Tx<T> given the element schema. */
  getTxSerializer(elementSchema: Schema): (value: unknown) => Uint8Array;
  /** Get a deserializer for Rx<T> given the element schema. */
  getRxDeserializer(elementSchema: Schema): (bytes: Uint8Array) => unknown;
}

/**
 * Bind all Tx/Rx channels found in the arguments.
 *
 * Walks the argument structure using the provided schema, finds any
 * Tx/Rx channels, allocates channel IDs for them, and binds them to
 * the registry.
 *
 * @param schemas - Schema for each argument
 * @param args - The actual argument values
 * @param allocator - Allocator for channel IDs
 * @param registry - Registry to bind channels to
 * @param serializers - Serializers for Tx/Rx element types
 */
export function bindChannels(
  schemas: Schema[],
  args: unknown[],
  allocator: ChannelIdAllocator,
  registry: ChannelRegistry,
  serializers: BindingSerializers,
): void {
  for (let i = 0; i < schemas.length; i++) {
    bindValue(schemas[i], args[i], allocator, registry, serializers);
  }
}

/**
 * Bind a single value according to its schema.
 */
function bindValue(
  schema: Schema,
  value: unknown,
  allocator: ChannelIdAllocator,
  registry: ChannelRegistry,
  serializers: BindingSerializers,
): void {
  switch (schema.kind) {
    // Primitives - nothing to bind
    case 'bool':
    case 'u8': case 'u16': case 'u32': case 'u64':
    case 'i8': case 'i16': case 'i32': case 'i64':
    case 'f32': case 'f64':
    case 'string':
    case 'bytes':
      return;

    case 'tx': {
      const tx = value as Tx<unknown>;
      if (!tx.isBound) {
        const channelId = allocator.next();
        const serialize = serializers.getTxSerializer(schema.element);
        tx.bind(channelId, registry, serialize);
      }
      return;
    }

    case 'rx': {
      const rx = value as Rx<unknown>;
      if (!rx.isBound) {
        const channelId = allocator.next();
        const deserialize = serializers.getRxDeserializer(schema.element);
        rx.bind(channelId, registry, deserialize);

        // Also bind the paired Tx if present
        if (rx._pair && !rx._pair.isBound) {
          const txSerialize = serializers.getTxSerializer(schema.element);
          rx._pair.bind(channelId, registry, txSerialize);
        }
      }
      return;
    }

    case 'vec': {
      const arr = value as unknown[];
      for (const item of arr) {
        bindValue(schema.element, item, allocator, registry, serializers);
      }
      return;
    }

    case 'option': {
      if (value !== null && value !== undefined) {
        bindValue(schema.inner, value, allocator, registry, serializers);
      }
      return;
    }

    case 'map': {
      const map = value as Map<unknown, unknown>;
      for (const [k, v] of map) {
        bindValue(schema.key, k, allocator, registry, serializers);
        bindValue(schema.value, v, allocator, registry, serializers);
      }
      return;
    }

    case 'struct': {
      const obj = value as Record<string, unknown>;
      for (const [fieldName, fieldSchema] of Object.entries(schema.fields)) {
        if (fieldName in obj) {
          bindValue(fieldSchema, obj[fieldName], allocator, registry, serializers);
        }
      }
      return;
    }

    case 'enum': {
      // Enum value should be { variant: string, fields: unknown[] }
      const enumVal = value as { variant: string; fields?: unknown[] };
      const variantSchemas = schema.variants[enumVal.variant];
      if (variantSchemas && enumVal.fields) {
        for (let i = 0; i < variantSchemas.length; i++) {
          bindValue(variantSchemas[i], enumVal.fields[i], allocator, registry, serializers);
        }
      }
      return;
    }

    default: {
      // Exhaustiveness check
      const _exhaustive: never = schema;
      throw new Error(`Unknown schema kind: ${(schema as Schema).kind}`);
    }
  }
}
