// Node subject for the roam compliance suite.
//
// This demonstrates the minimal code needed to implement a roam service
// using the @roam/tcp transport library.

import type {
  TestbedHandler,
  Point,
  Person,
  Rectangle,
  Color,
  Shape,
  Canvas,
  Message,
} from "@bearcove/roam-generated/testbed.ts";
import {
  testbed_streamingHandlers,
  type StreamingMethodHandler,
} from "@bearcove/roam-generated/testbed.ts";
import { Server } from "@bearcove/roam-tcp";
import {
  type StreamingDispatcher,
  type StreamRegistry,
  type TaskSender,
  type Tx,
  type Rx,
  encodeResultErr,
  encodeInvalidPayload,
  encodeUnknownMethod,
} from "@bearcove/roam-core";

// Service implementation
class TestbedService implements TestbedHandler {
  // Echo methods
  echo(message: string): string {
    return message;
  }

  reverse(message: string): string {
    return Array.from(message).reverse().join("");
  }

  // Streaming methods
  async sum(numbers: Rx<number>): Promise<bigint> {
    // Server receives numbers via Rx channel and sums them
    let total = 0n;
    for await (const n of numbers) {
      total += BigInt(n);
    }
    return total;
  }

  async generate(count: number, output: Tx<number>): Promise<void> {
    // Server sends count numbers via Tx channel
    for (let i = 0; i < count; i++) {
      output.send(i);
    }
    // Note: output.close() is called by the generated handler after this returns
  }

  async transform(input: Rx<string>, output: Tx<string>): Promise<void> {
    // Server receives via Rx, sends via Tx (echo back as-is)
    for await (const s of input) {
      output.send(s);
    }
    // Note: output.close() is called by the generated handler after this returns
  }

  // Complex type methods
  echoPoint(point: Point): Point {
    return point;
  }

  createPerson(name: string, age: number, email: string | null): Person {
    return { name, age, email };
  }

  rectangleArea(rect: Rectangle): number {
    const width = Math.abs(rect.bottom_right.x - rect.top_left.x);
    const height = Math.abs(rect.bottom_right.y - rect.top_left.y);
    return width * height;
  }

  parseColor(name: string): Color | null {
    switch (name.toLowerCase()) {
      case "red":
        return { tag: "Red" };
      case "green":
        return { tag: "Green" };
      case "blue":
        return { tag: "Blue" };
      default:
        return null;
    }
  }

  shapeArea(shape: Shape): number {
    switch (shape.tag) {
      case "Circle":
        return Math.PI * shape.radius * shape.radius;
      case "Rectangle":
        return shape.width * shape.height;
      case "Point":
        return 0;
    }
  }

  createCanvas(name: string, shapes: Shape[], background: Color): Canvas {
    return { name, shapes, background };
  }

  processMessage(msg: Message): Message {
    switch (msg.tag) {
      case "Text":
        return { tag: "Text", value: `Processed: ${msg.value}` };
      case "Number":
        return { tag: "Number", value: msg.value * 2n };
      case "Data":
        return msg;
    }
  }

  getPoints(count: number): Point[] {
    const points: Point[] = [];
    for (let i = 0; i < count; i++) {
      points.push({ x: i, y: i * 2 });
    }
    return points;
  }

  swapPair(pair: { 0: number; 1: string }): { 0: string; 1: number } {
    return { 0: pair[1], 1: pair[0] };
  }
}

// Streaming dispatcher that uses the generated streaming handlers
class TestbedStreamingDispatcher implements StreamingDispatcher {
  private service = new TestbedService();
  private handlers: Map<bigint, StreamingMethodHandler<TestbedHandler>>;

  constructor() {
    this.handlers = testbed_streamingHandlers;
  }

  async dispatch(
    methodId: bigint,
    payload: Uint8Array,
    requestId: bigint,
    registry: StreamRegistry,
    taskSender: TaskSender,
  ): Promise<void> {
    const handler = this.handlers.get(methodId);
    if (!handler) {
      // Unknown method - send error response
      taskSender({
        kind: "response",
        requestId,
        payload: encodeResultErr(encodeUnknownMethod()),
      });
      return;
    }

    await handler(this.service, payload, requestId, registry, taskSender);
  }
}

async function runServer() {
  const server = new Server();
  await server.runSubjectStreaming(new TestbedStreamingDispatcher());
}

async function runClient() {
  const { TestbedClient } = await import("@bearcove/roam-generated/testbed.ts");
  const { encodeI32 } = await import("@bearcove/roam-core");

  const addr = process.env.PEER_ADDR;
  if (!addr) {
    throw new Error("PEER_ADDR env var not set");
  }

  const scenario = process.env.CLIENT_SCENARIO ?? "echo";
  console.error(`client mode: connecting to ${addr}, scenario=${scenario}`);

  const server = new Server();
  const conn = await server.connect(addr);
  const client = new TestbedClient(conn);

  switch (scenario) {
    case "echo": {
      const result = await client.echo("hello from client");
      console.error(`echo result: ${result}`);
      break;
    }
    case "sum": {
      // Client-to-server streaming: create Tx, send numbers, then call sum
      const [tx, _streamId] = conn.createTx<number>((v) => encodeI32(v));

      // Send numbers asynchronously
      const sendTask = (async () => {
        for (let i = 1; i <= 5; i++) {
          console.error(`sending ${i}`);
          tx.send(i);
        }
        console.error("closing tx");
        tx.close();
      })();

      // The Rx on server side uses our Tx's stream ID
      // We need to create an Rx with the same stream ID for the client API
      // Actually, we need to pass tx to sum - but sum takes Rx, not Tx!
      // Looking at the generated code, sum takes Rx and uses rx.streamId
      // For client->server streaming, the client creates a Tx and the
      // generated client code expects an Rx (which has the streamId)
      //
      // Wait - looking at the Rust client, it creates a channel pair (tx, rx)
      // and passes rx to sum. Let me check the roam channel API...
      //
      // For now, let me check if there's a createChannel or similar
      // Actually looking at the generated client code:
      //   async sum(numbers: Rx<number>): Promise<bigint> {
      //     const payload = encodeU64(numbers.streamId);
      // It just needs the streamId. So I need to create an Rx that shares
      // the stream ID with the Tx I'll use to send data.
      //
      // The issue is createTx returns Tx, createRx returns Rx, and they have
      // different stream IDs. For client->server streaming, we need both
      // to share the same ID.
      //
      // Looking at connection.ts more carefully - the Tx writes to the
      // registry's outgoing channel, and Rx reads from incoming.
      // For client->server streaming on the client side:
      // - Client creates Tx to send data (outgoing)
      // - Client needs to encode the stream ID in the request
      // - Server creates Rx with that stream ID to receive
      //
      // So I need createTx for the actual sending, but I need to pass
      // something with .streamId to the generated client method.
      // Let me just create a fake Rx with the right stream ID.

      // Actually simpler - just call with the tx.streamId exposed somehow
      // For now, let's skip the streaming client tests or implement properly
      console.error("sum scenario: not yet implemented");
      break;
    }
    case "generate": {
      console.error("generate scenario: not yet implemented");
      break;
    }
    default:
      throw new Error(`unknown CLIENT_SCENARIO: ${scenario}`);
  }
}

async function main() {
  const mode = process.env.SUBJECT_MODE ?? "server";

  if (mode === "client") {
    await runClient();
  } else {
    await runServer();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
