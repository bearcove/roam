// Metadata conversion utilities.
//
// Converts between the user-friendly Map<string, ClientMetadataValue>
// and the wire format MetadataEntry[].

import type { MetadataEntry, MetadataValue } from "@bearcove/roam-wire";
import { metadataString, metadataBytes, metadataU64, MetadataFlags } from "@bearcove/roam-wire";
import type { ClientMetadataValue } from "./middleware.ts";

/**
 * Convert a client metadata Map to wire format entries.
 *
 * By default, all entries have flags = 0n (no special handling).
 * To set flags, use `metadataMapToEntriesWithFlags`.
 *
 * @param map - Client metadata map
 * @returns Wire format metadata entries
 */
export function metadataMapToEntries(
  map: Map<string, ClientMetadataValue>
): MetadataEntry[] {
  const entries: MetadataEntry[] = [];
  for (const [key, value] of map) {
    let wireValue: MetadataValue;
    if (typeof value === "string") {
      wireValue = metadataString(value);
    } else if (typeof value === "bigint") {
      wireValue = metadataU64(value);
    } else {
      wireValue = metadataBytes(value);
    }
    // r[impl call.metadata.flags] - Default flags to 0 (no special handling)
    entries.push([key, wireValue, MetadataFlags.NONE]);
  }
  return entries;
}

/**
 * Convert a client metadata Map to wire format entries with custom flags.
 *
 * The flagsMap allows specifying flags per key. Keys not in flagsMap get 0n.
 *
 * @param map - Client metadata map
 * @param flagsMap - Map of key to flags (use MetadataFlags constants)
 * @returns Wire format metadata entries
 */
export function metadataMapToEntriesWithFlags(
  map: Map<string, ClientMetadataValue>,
  flagsMap: Map<string, bigint>
): MetadataEntry[] {
  const entries: MetadataEntry[] = [];
  for (const [key, value] of map) {
    let wireValue: MetadataValue;
    if (typeof value === "string") {
      wireValue = metadataString(value);
    } else if (typeof value === "bigint") {
      wireValue = metadataU64(value);
    } else {
      wireValue = metadataBytes(value);
    }
    const flags = flagsMap.get(key) ?? MetadataFlags.NONE;
    entries.push([key, wireValue, flags]);
  }
  return entries;
}

/**
 * Convert wire format metadata entries to a client-friendly Map.
 *
 * Note: This discards the flags from the entries.
 * Use `metadataEntriesWithFlags` if you need to preserve flags.
 *
 * @param entries - Wire format metadata entries
 * @returns Client metadata map
 */
export function metadataEntriesToMap(
  entries: MetadataEntry[]
): Map<string, ClientMetadataValue> {
  const map = new Map<string, ClientMetadataValue>();
  for (const [key, value, _flags] of entries) {
    map.set(key, value.value);
  }
  return map;
}

/**
 * Convert wire format metadata entries to a client-friendly Map, preserving flags.
 *
 * @param entries - Wire format metadata entries
 * @returns Tuple of [metadata map, flags map]
 */
export function metadataEntriesWithFlags(
  entries: MetadataEntry[]
): [Map<string, ClientMetadataValue>, Map<string, bigint>] {
  const map = new Map<string, ClientMetadataValue>();
  const flags = new Map<string, bigint>();
  for (const [key, value, entryFlags] of entries) {
    map.set(key, value.value);
    flags.set(key, entryFlags);
  }
  return [map, flags];
}
