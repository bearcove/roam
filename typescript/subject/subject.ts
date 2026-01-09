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
import { testbed_methodHandlers } from "@bearcove/roam-generated/testbed.ts";
import { Server, type ServiceDispatcher } from "@bearcove/roam-tcp";
import { UnaryDispatcher, type Tx, type Rx } from "@bearcove/roam-core";

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
  sum(_numbers: Rx<number>): bigint {
    // TODO: Implement streaming - for now return 0
    // Server receives numbers via Rx channel and sums them
    return 0n;
  }

  generate(_count: number, _output: Tx<number>): void {
    // TODO: Implement streaming - for now do nothing
    // Server sends count numbers via Tx channel
  }

  transform(_input: Rx<string>, _output: Tx<string>): void {
    // TODO: Implement streaming - for now do nothing
    // Server receives via Rx, sends via Tx
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

// Dispatcher wraps the generated dispatch function
class TestbedDispatcher implements ServiceDispatcher {
  private service = new TestbedService();
  private dispatcher = new UnaryDispatcher(testbed_methodHandlers);

  async dispatchUnary(methodId: bigint, payload: Uint8Array): Promise<Uint8Array> {
    return this.dispatcher.dispatch(this.service, methodId, payload);
  }
}

async function main() {
  const server = new Server();
  await server.runSubject(new TestbedDispatcher());
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
