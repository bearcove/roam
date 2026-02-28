import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { encodeU16, encodeU32, encodeU64 } from "@bearcove/roam-postcard";
import {
  type Message,
  MetadataFlags,
  parityOdd,
  parityEven,
  connectionSettings,
  helloV7,
  helloYourself,
  metadataString,
  metadataBytes,
  metadataU64,
  metadataEntry,
  messageHello,
  messageHelloYourself,
  messageProtocolError,
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
  encodeMessage,
  decodeMessage,
} from "./index.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

function loadGoldenVector(name: string): Uint8Array {
  const projectRoot = join(__dirname, "..", "..", "..", "..");
  const vectorPath = join(projectRoot, "test-fixtures", "golden-vectors", "wire-v7", `${name}.bin`);
  return new Uint8Array(readFileSync(vectorPath));
}

function sampleMetadata() {
  return [
    metadataEntry("trace-id", metadataString("abc123"), MetadataFlags.NONE),
    metadataEntry(
      "auth",
      metadataBytes(new Uint8Array([0xde, 0xad, 0xbe, 0xef])),
      { 0: MetadataFlags.SENSITIVE[0] | MetadataFlags.NO_PROPAGATE[0] },
    ),
    metadataEntry("attempt", metadataU64(2n), MetadataFlags.NONE),
  ];
}

function expectedMessages(): Array<[name: string, message: Message]> {
  const meta = sampleMetadata();

  return [
    ["message_hello", messageHello(helloV7(parityOdd(), 64, meta))],
    [
      "message_hello_yourself",
      messageHelloYourself(helloYourself(parityEven(), 32, meta)),
    ],
    ["message_protocol_error", messageProtocolError("bad frame sequence")],
    ["message_connection_open", messageConnect(2n, connectionSettings(parityOdd(), 64), meta)],
    ["message_connection_accept", messageAccept(2n, connectionSettings(parityEven(), 96), meta)],
    ["message_connection_reject", messageReject(4n, meta)],
    ["message_connection_close", messageGoodbye(2n, meta)],
    [
      "message_request_call",
      messageRequest(11n, 0xE5A1_D6B2_C390_F001n, encodeU32(0x1234_5678), meta, [3n, 5n], 2n),
    ],
    [
      "message_request_response",
      messageResponse(11n, encodeU64(0xFACE_B00Cn), meta, [7n], 2n),
    ],
    ["message_request_cancel", messageCancel(11n, 2n, meta)],
    ["message_channel_item", messageData(3n, encodeU16(77), 2n)],
    ["message_channel_close", messageClose(3n, 2n, meta)],
    ["message_channel_reset", messageReset(3n, 2n, meta)],
    ["message_channel_grant_credit", messageCredit(3n, 1024, 2n)],
  ];
}

describe("wire-v7 golden vectors", () => {
  it("encodes bytes matching Rust fixtures", () => {
    for (const [name, message] of expectedMessages()) {
      const encoded = encodeMessage(message);
      const expected = loadGoldenVector(name);
      expect(Array.from(encoded), name).toEqual(Array.from(expected));
    }
  });

  it("decodes Rust fixtures into expected messages", () => {
    for (const [name, expectedMessage] of expectedMessages()) {
      const bytes = loadGoldenVector(name);
      const decoded = decodeMessage(bytes);
      expect(decoded.next, name).toBe(bytes.length);
      expect(decoded.value, name).toEqual(expectedMessage);
    }
  });
});
