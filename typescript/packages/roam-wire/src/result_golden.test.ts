// Golden vector tests for Result<T, RoamError<E>> postcard encoding.
//
// These verify that the TypeScript decode logic matches the canonical
// Rust encoding produced by generate_golden_vectors.

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { decodeVarintNumber, decodeString, decodeU32 } from "@bearcove/roam-postcard";
import { RpcError, RpcErrorCode } from "./rpc_error.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

function loadVector(path: string): Uint8Array {
  const projectRoot = join(__dirname, "..", "..", "..", "..");
  return new Uint8Array(readFileSync(join(projectRoot, "test-fixtures", "golden-vectors", path)));
}

// Mirrors the decode logic in connection.ts
function decodeOkString(bytes: Uint8Array): string {
  if (bytes[0] !== 0) throw new Error("expected Ok");
  return decodeString(bytes, 1).value;
}

function decodeOkU32(bytes: Uint8Array): number {
  if (bytes[0] !== 0) throw new Error("expected Ok");
  return decodeU32(bytes, 1).value;
}

function decodeErr(bytes: Uint8Array): RpcError {
  if (bytes[0] !== 1) throw new Error("expected Err");
  const variant = decodeVarintNumber(bytes, 1);
  if (variant.value === RpcErrorCode.USER) {
    return new RpcError(RpcErrorCode.USER, bytes.slice(variant.next));
  }
  return new RpcError(variant.value as RpcErrorCode);
}

describe("Result/RoamError golden vectors", () => {
  it("ok_string: [0x00, len, ...bytes]", () => {
    const bytes = loadVector("result/ok_string.bin");
    expect(Array.from(bytes)).toEqual([0x00, 0x05, 0x68, 0x65, 0x6c, 0x6c, 0x6f]);
    expect(decodeOkString(bytes)).toBe("hello");
  });

  it("ok_u32: [0x00, varint(42)]", () => {
    const bytes = loadVector("result/ok_u32.bin");
    expect(Array.from(bytes)).toEqual([0x00, 0x2a]);
    expect(decodeOkU32(bytes)).toBe(42);
  });

  it("err_unknown_method: [0x01, 0x01]", () => {
    const bytes = loadVector("result/err_unknown_method.bin");
    expect(Array.from(bytes)).toEqual([0x01, 0x01]);
    const err = decodeErr(bytes);
    expect(err.code).toBe(RpcErrorCode.UNKNOWN_METHOD);
    expect(err.payload).toBeNull();
  });

  it("err_invalid_payload: [0x01, 0x02]", () => {
    const bytes = loadVector("result/err_invalid_payload.bin");
    expect(Array.from(bytes)).toEqual([0x01, 0x02]);
    const err = decodeErr(bytes);
    expect(err.code).toBe(RpcErrorCode.INVALID_PAYLOAD);
    expect(err.payload).toBeNull();
  });

  it("err_cancelled: [0x01, 0x03]", () => {
    const bytes = loadVector("result/err_cancelled.bin");
    expect(Array.from(bytes)).toEqual([0x01, 0x03]);
    const err = decodeErr(bytes);
    expect(err.code).toBe(RpcErrorCode.CANCELLED);
    expect(err.payload).toBeNull();
  });

  it("err_user_string: [0x01, 0x00, len, ...bytes]", () => {
    const bytes = loadVector("result/err_user_string.bin");
    expect(Array.from(bytes)).toEqual([0x01, 0x00, 0x04, 0x6f, 0x6f, 0x70, 0x73]);
    const err = decodeErr(bytes);
    expect(err.code).toBe(RpcErrorCode.USER);
    expect(err.payload).not.toBeNull();
    // payload starts at offset 2 (after Err disc + User disc)
    expect(decodeString(err.payload!, 0).value).toBe("oops");
  });
});
