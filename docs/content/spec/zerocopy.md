+++
title = "Zero-copy"
sort_by = "weight"
weight = 6
insert_anchor_links = "left"
+++

# Zero-copy data flow

r[zerocopy]

Roam aims to minimize copies at every layer. This document specifies how
payloads flow through the system — from user code, through serialization
and transport, to deserialization on the other side — and which link types
enable zero-copy at each stage.

## Backing storage

r[zerocopy.backing]

A **Backing** is an owned handle that keeps a region of bytes alive. The
receive path deserializes borrowing from a Backing, producing values that
may contain `&str` or `&[u8]` pointing into the original buffer.

r[zerocopy.backing.boxed]

**Boxed** — a heap-allocated `Box<[u8]>`. Used by stream (TCP) and
WebSocket links after reading into an owned buffer.

r[zerocopy.backing.bipbuf]

**BipBuf** — a region inside a shared-memory BipBuffer. For small messages
that fit inline in the ring, the receiver borrows directly from the ring
and returns the region when done.

r[zerocopy.backing.varslot]

**VarSlot** — a slot in a shared-memory VarSlotPool. For medium messages,
the sender writes into a variable-size slot and the receiver borrows from
it. The slot is returned to the pool when the Backing is dropped.

r[zerocopy.backing.mmap]

**Mmap** — a memory-mapped region for large payloads. The sender writes a
file (or anonymous mapping), the receiver maps it and borrows from the
mapping.

## Send path

r[zerocopy.send]

On the send side, user code provides a value to serialize. The value may
borrow from the calling context.

### Borrowed arguments

r[zerocopy.send.borrowed]

A call like `client.method(&buf[..12]).await` passes a borrowed slice.
The borrow is valid for the duration of the `.await` — the future
captures the reference and serialization completes before the future
resolves.

r[zerocopy.send.borrowed-in-struct]

A call like `client.method(Context { name: &my_string }).await` passes a
struct that borrows from the calling scope. This is valid for the same
reason: the future holds the struct, which holds the borrow, and
serialization happens within the future's lifetime.

r[zerocopy.send.serialize-before-yield]

Serialization of arguments MUST complete before the send future yields
for the first time. This ensures borrowed data remains valid — the caller
only needs to hold borrows until the first `.await` suspension point.

### Link-specific send behavior

r[zerocopy.send.stream]

**Stream links (TCP):** serialize into a write buffer, flush to socket.
One copy (value → write buffer).

r[zerocopy.send.websocket]

**WebSocket links:** serialize into a message buffer, send as a WebSocket
frame. One copy (value → message buffer).

r[zerocopy.send.shm]

**SHM links:** serialize directly into the BipBuffer or VarSlot. The
`LinkTx::alloc` call returns a `WriteSlot` pointing into shared memory,
and serialization writes directly into it. Zero copies for the payload
bytes — the receiver reads from the same physical memory.

## Receive path

r[zerocopy.recv]

On the receive side, `LinkRx::recv` returns a `Backing` that owns the
raw bytes. The conduit deserializes borrowing from this backing, producing
a `SelfRef<T>` that pairs the decoded value with its backing.

r[zerocopy.recv.selfref]

`SelfRef<T>` guarantees correct drop order: the decoded value is dropped
before its backing storage. This allows the value to contain references
(`&str`, `&[u8]`) pointing into the backing without use-after-free.

### Link-specific receive behavior

r[zerocopy.recv.stream]

**Stream links (TCP):** `recv` reads a length-prefixed frame into a
`Box<[u8]>`. One copy (socket → heap). Deserialization borrows from the
box.

r[zerocopy.recv.websocket]

**WebSocket links:** `recv` receives a complete message as `bytes::Bytes`,
converted to `Box<[u8]>`. One copy. Deserialization borrows from the box.

r[zerocopy.recv.shm.inline]

**SHM links (inline):** for messages below the inline threshold, `recv`
returns a Backing that borrows from the BipBuffer ring. Zero copies —
deserialization reads directly from shared memory.

r[zerocopy.recv.shm.slotref]

**SHM links (slot-ref):** for medium messages, `recv` returns a Backing
that borrows from a VarSlot. Zero copies — deserialization reads from the
slot, which is returned to the pool when the Backing drops.

r[zerocopy.recv.shm.mmap]

**SHM links (mmap):** for large payloads, `recv` maps the region and
returns a Backing that owns the mapping. Zero copies — deserialization
reads from the mapping.

## Payload representation

r[zerocopy.payload]

`Payload` represents a value ready for serialization. Its variants
reflect the different ownership situations:

r[zerocopy.payload.borrowed]

**Borrowed** — a type-erased pointer to a value in the caller's stack
frame plus its Shape. Used on the send path when the value is borrowed
from user code.

r[zerocopy.payload.bytes]

**Bytes** — a contiguous byte buffer that is already serialized (e.g.
when forwarding a message without deserializing, or when the link
provides raw bytes). Paired with a Backing to keep the buffer alive.

## Copy count summary

r[zerocopy.copies]

| Direction | Stream (TCP) | WebSocket | SHM (inline) | SHM (slot-ref) | SHM (mmap) |
|-----------|-------------|-----------|--------------|-----------------|------------|
| Send      | 1           | 1         | 0            | 0               | 0          |
| Receive   | 1           | 1         | 0            | 0               | 0          |
