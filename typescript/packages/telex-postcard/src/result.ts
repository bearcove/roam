export type TelexErrorPayload =
  | { tag: "User"; value: Uint8Array }
  | { tag: "UnknownMethod" }
  | { tag: "InvalidPayload" }
  | { tag: "Cancelled" };
