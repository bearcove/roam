//! Experiment: Testing container-level proxy for Tx/Rx serialization
//!
//! Goal: `Tx<T>` and `Rx<T>` should:
//! - Have pokeable `stream_id` field (for Connection to set it)
//! - Have opaque fields for sender/receiver (they don't implement Facet)
//! - Serialize as just a u64 via container-level proxy
//!
//! Results:
//! 1. Partial opaque (only on non-Facet fields) works ✅
//! 2. Can poke the stream_id field ✅
//! 3. Container-level proxy for serialization - TESTING

use std::marker::PhantomData;

use facet::Facet;
use facet_reflect::Poke;
use tokio::sync::mpsc;

// ============================================================================
// Experiment A: Newtype proxy (TxProxy)
// ============================================================================

#[derive(Facet, Clone, PartialEq)]
#[facet(transparent)]
pub struct TxProxy(pub u64);

/// Tx stream handle - serializes as just a u64 via TxProxy newtype
#[derive(Facet)]
#[facet(proxy = TxProxy)]
pub struct Tx<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: mpsc::Sender<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// For SERIALIZATION: &Tx<T> -> TxProxy
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<&Tx<T>> for TxProxy {
    type Error = std::convert::Infallible;
    fn try_from(tx: &Tx<T>) -> Result<Self, Self::Error> {
        Ok(TxProxy(tx.stream_id))
    }
}

// For DESERIALIZATION: TxProxy -> Tx<T>
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<TxProxy> for Tx<T> {
    type Error = std::convert::Infallible;
    fn try_from(proxy: TxProxy) -> Result<Self, Self::Error> {
        let (sender, _rx) = mpsc::channel(1); // placeholder
        Ok(Tx {
            stream_id: proxy.0,
            sender,
            _marker: PhantomData,
        })
    }
}

// ============================================================================
// Experiment B: Direct u64 proxy (no newtype)
// ============================================================================

/// Tx2 stream handle - serializes as just a u64 directly
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Tx2<T: 'static> {
    pub stream_id: u64,
    #[facet(opaque)]
    sender: mpsc::Sender<Vec<u8>>,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// For SERIALIZATION: &Tx2<T> -> u64
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<&Tx2<T>> for u64 {
    type Error = std::convert::Infallible;
    fn try_from(tx: &Tx2<T>) -> Result<Self, Self::Error> {
        Ok(tx.stream_id)
    }
}

// For DESERIALIZATION: u64 -> Tx2<T>
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<u64> for Tx2<T> {
    type Error = std::convert::Infallible;
    fn try_from(stream_id: u64) -> Result<Self, Self::Error> {
        let (sender, _rx) = mpsc::channel(1); // placeholder
        Ok(Tx2 {
            stream_id,
            sender,
            _marker: PhantomData,
        })
    }
}

// ============================================================================
// Test helpers
// ============================================================================

fn inspect_shape<T: Facet<'static>>(name: &str) {
    let shape = T::SHAPE;
    println!("\n=== {name} ===");
    println!("type_identifier: {}", shape.type_identifier);
    println!("module_path: {:?}", shape.module_path);
    println!(
        "type_params: {:?}",
        shape.type_params.iter().map(|p| p.name).collect::<Vec<_>>()
    );

    if let Some(proxy) = shape.proxy {
        println!("proxy: {} (container-level)", proxy.shape.type_identifier);
    } else {
        println!("proxy: None");
    }

    if let facet::Type::User(facet::UserType::Struct(sk)) = shape.ty {
        println!("fields ({}):", sk.fields.len());
        for field in sk.fields {
            let proxy_info = if field.proxy.is_some() {
                " [field proxy]"
            } else {
                ""
            };
            println!(
                "  - {}: {} (shape: {}){}",
                field.name,
                field.shape().type_identifier,
                field.shape(),
                proxy_info
            );
        }
    } else {
        println!("ty: {:?}", shape.ty);
    }
}

fn test_poke_tx() {
    println!("\n### Testing Poke on Tx (newtype proxy) ###");

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let mut tx = Tx::<i32> {
        stream_id: 0,
        sender,
        _marker: PhantomData,
    };

    println!("Initial stream_id: {}", tx.stream_id);

    let poke = Poke::new(&mut tx);

    match poke.into_struct() {
        Ok(mut poke_struct) => match poke_struct.field_by_name("stream_id") {
            Ok(mut field_poke) => match field_poke.set(42u64) {
                Ok(()) => println!("✅ Successfully poked stream_id = 42"),
                Err(e) => println!("❌ Failed to set value: {e}"),
            },
            Err(e) => println!("❌ Cannot access field 'stream_id': {e}"),
        },
        Err(e) => println!("❌ Cannot convert to PokeStruct: {e}"),
    }

    println!("Final stream_id: {}", tx.stream_id);
}

fn test_poke_tx2() {
    println!("\n### Testing Poke on Tx2 (direct u64 proxy) ###");

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let mut tx = Tx2::<i32> {
        stream_id: 0,
        sender,
        _marker: PhantomData,
    };

    println!("Initial stream_id: {}", tx.stream_id);

    let poke = Poke::new(&mut tx);

    match poke.into_struct() {
        Ok(mut poke_struct) => match poke_struct.field_by_name("stream_id") {
            Ok(mut field_poke) => match field_poke.set(42u64) {
                Ok(()) => println!("✅ Successfully poked stream_id = 42"),
                Err(e) => println!("❌ Failed to set value: {e}"),
            },
            Err(e) => println!("❌ Cannot access field 'stream_id': {e}"),
        },
        Err(e) => println!("❌ Cannot convert to PokeStruct: {e}"),
    }

    println!("Final stream_id: {}", tx.stream_id);
}

fn test_json_roundtrip() {
    println!("\n### Testing JSON Roundtrip (Tx with newtype proxy) ###");

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let original = Tx::<i32> {
        stream_id: 42,
        sender,
        _marker: PhantomData,
    };

    println!("Original stream_id: {}", original.stream_id);

    match facet_json::to_string(&original) {
        Ok(json) => {
            println!("Serialized JSON: {}", json);
            match facet_json::from_str::<Tx<i32>>(&json) {
                Ok(deserialized) => {
                    println!("Deserialized stream_id: {}", deserialized.stream_id);
                    if original.stream_id == deserialized.stream_id {
                        println!("✅ JSON roundtrip successful!");
                    } else {
                        println!("❌ JSON roundtrip failed - stream_ids differ");
                    }
                }
                Err(e) => println!("❌ JSON deserialization failed: {e}"),
            }
        }
        Err(e) => println!("❌ JSON serialization failed: {e}"),
    }
}

fn test_json_roundtrip_tx2() {
    println!("\n### Testing JSON Roundtrip (Tx2 with direct u64 proxy) ###");

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let original = Tx2::<i32> {
        stream_id: 42,
        sender,
        _marker: PhantomData,
    };

    println!("Original stream_id: {}", original.stream_id);

    match facet_json::to_string(&original) {
        Ok(json) => {
            println!("Serialized JSON: {}", json);
            match facet_json::from_str::<Tx2<i32>>(&json) {
                Ok(deserialized) => {
                    println!("Deserialized stream_id: {}", deserialized.stream_id);
                    if original.stream_id == deserialized.stream_id {
                        println!("✅ JSON roundtrip successful!");
                    } else {
                        println!("❌ JSON roundtrip failed - stream_ids differ");
                    }
                }
                Err(e) => println!("❌ JSON deserialization failed: {e}"),
            }
        }
        Err(e) => println!("❌ JSON serialization failed: {e}"),
    }
}

fn test_postcard_roundtrip() {
    println!("\n### Testing Postcard Roundtrip ###");

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let original = Tx::<i32> {
        stream_id: 42,
        sender,
        _marker: PhantomData,
    };

    println!("Original stream_id: {}", original.stream_id);

    // Serialize
    match facet_postcard::to_vec(&original) {
        Ok(bytes) => {
            println!("Serialized bytes: {:?} ({} bytes)", bytes, bytes.len());

            // Deserialize
            match facet_postcard::from_slice::<Tx<i32>>(&bytes) {
                Ok(deserialized) => {
                    println!("Deserialized stream_id: {}", deserialized.stream_id);
                    if original.stream_id == deserialized.stream_id {
                        println!("✅ Postcard roundtrip successful!");
                    } else {
                        println!("❌ Postcard roundtrip failed - stream_ids differ");
                    }
                }
                Err(e) => println!("❌ Postcard deserialization failed: {e}"),
            }
        }
        Err(e) => println!("❌ Postcard serialization failed: {e}"),
    }
}

fn test_tx_in_struct() {
    println!("\n### Testing Tx embedded in a struct ###");

    // This is how Tx would appear in RPC arguments
    #[derive(Facet)]
    struct StreamRequest {
        request_id: u64,
        data_stream: Tx<String>,
    }

    let (sender, _rx) = mpsc::channel::<Vec<u8>>(1);
    let request = StreamRequest {
        request_id: 100,
        data_stream: Tx {
            stream_id: 42,
            sender,
            _marker: PhantomData,
        },
    };

    println!(
        "Original: request_id={}, data_stream.stream_id={}",
        request.request_id, request.data_stream.stream_id
    );

    // JSON
    match facet_json::to_string(&request) {
        Ok(json) => {
            println!("JSON: {}", json);
            match facet_json::from_str::<StreamRequest>(&json) {
                Ok(deser) => {
                    println!(
                        "✅ JSON roundtrip: request_id={}, data_stream.stream_id={}",
                        deser.request_id, deser.data_stream.stream_id
                    );
                }
                Err(e) => println!("❌ JSON deser failed: {e}"),
            }
        }
        Err(e) => println!("❌ JSON ser failed: {e}"),
    }

    // Postcard
    let (sender2, _rx2) = mpsc::channel::<Vec<u8>>(1);
    let request2 = StreamRequest {
        request_id: 100,
        data_stream: Tx {
            stream_id: 42,
            sender: sender2,
            _marker: PhantomData,
        },
    };

    match facet_postcard::to_vec(&request2) {
        Ok(bytes) => {
            println!("Postcard bytes: {:?} ({} bytes)", bytes, bytes.len());
            match facet_postcard::from_slice::<StreamRequest>(&bytes) {
                Ok(deser) => {
                    println!(
                        "✅ Postcard roundtrip: request_id={}, data_stream.stream_id={}",
                        deser.request_id, deser.data_stream.stream_id
                    );
                }
                Err(e) => println!("❌ Postcard deser failed: {e}"),
            }
        }
        Err(e) => println!("❌ Postcard ser failed: {e}"),
    }
}

// ============================================================================
// Experiment C: Wrapper for Option<Receiver> that implements Facet
// ============================================================================

/// A wrapper around `Option<mpsc::Receiver<Vec<u8>>>` that implements Facet.
/// This allows Poke::get_mut() to work, enabling .take() via reflection.
#[derive(Facet)]
#[facet(opaque)] // Still opaque for serialization (shouldn't be serialized)
pub struct ReceiverSlot {
    inner: Option<mpsc::Receiver<Vec<u8>>>,
}

impl ReceiverSlot {
    pub fn new(receiver: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            inner: Some(receiver),
        }
    }

    pub fn empty() -> Self {
        Self { inner: None }
    }

    pub fn take(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.inner.take()
    }

    pub fn is_some(&self) -> bool {
        self.inner.is_some()
    }
}

/// Rx3 - uses ReceiverSlot wrapper so we can .take() via Poke
#[derive(Facet)]
#[facet(proxy = u64)]
pub struct Rx3<T: 'static> {
    pub stream_id: u64,
    pub receiver: ReceiverSlot, // Now pokeable!
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

// For SERIALIZATION: &Rx3<T> -> u64
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<&Rx3<T>> for u64 {
    type Error = std::convert::Infallible;
    fn try_from(rx: &Rx3<T>) -> Result<Self, Self::Error> {
        Ok(rx.stream_id)
    }
}

// For DESERIALIZATION: u64 -> Rx3<T>
#[allow(clippy::infallible_try_from)]
impl<T: 'static> TryFrom<u64> for Rx3<T> {
    type Error = std::convert::Infallible;
    fn try_from(stream_id: u64) -> Result<Self, Self::Error> {
        Ok(Rx3 {
            stream_id,
            receiver: ReceiverSlot::empty(),
            _marker: PhantomData,
        })
    }
}

fn test_poke_take_receiver() {
    println!("\n### Testing Poke + take() on Rx3 ###");

    // Create a channel
    let (_tx, rx) = mpsc::channel::<Vec<u8>>(64);

    // Create Rx3 with the receiver
    let mut rx3 = Rx3::<i32> {
        stream_id: 0,
        receiver: ReceiverSlot::new(rx),
        _marker: PhantomData,
    };

    println!(
        "Initial: stream_id={}, receiver.is_some()={}",
        rx3.stream_id,
        rx3.receiver.is_some()
    );

    // Now use Poke to:
    // 1. Set stream_id
    // 2. Take the receiver

    let poke = Poke::new(&mut rx3);

    match poke.into_struct() {
        Ok(mut poke_struct) => {
            // Step 1: Set stream_id
            match poke_struct.field_by_name("stream_id") {
                Ok(mut field_poke) => match field_poke.set(42u64) {
                    Ok(()) => println!("✅ Poked stream_id = 42"),
                    Err(e) => println!("❌ Failed to set stream_id: {e}"),
                },
                Err(e) => println!("❌ Cannot access stream_id: {e}"),
            }

            // Step 2: Get mutable reference to ReceiverSlot and .take()
            match poke_struct.field_by_name("receiver") {
                Ok(mut field_poke) => match field_poke.get_mut::<ReceiverSlot>() {
                    Ok(slot) => {
                        let taken = slot.take();
                        if taken.is_some() {
                            println!("✅ Successfully took receiver via Poke!");
                        } else {
                            println!("❌ Receiver was already None");
                        }
                    }
                    Err(e) => println!("❌ Failed to get_mut ReceiverSlot: {e}"),
                },
                Err(e) => println!("❌ Cannot access receiver field: {e}"),
            }
        }
        Err(e) => println!("❌ Cannot convert to PokeStruct: {e}"),
    }

    println!(
        "Final: stream_id={}, receiver.is_some()={}",
        rx3.stream_id,
        rx3.receiver.is_some()
    );
}

fn main() {
    println!("===========================================");
    println!("Container-Level Proxy Experiment");
    println!("===========================================");

    // Inspect shapes
    inspect_shape::<TxProxy>("TxProxy (newtype)");
    inspect_shape::<Tx<i32>>("Tx<i32> (newtype proxy)");
    inspect_shape::<Tx2<i32>>("Tx2<i32> (direct u64 proxy)");

    // Test poking (Connection needs this to set stream_id)
    test_poke_tx();
    test_poke_tx2();

    // Test serialization roundtrips
    test_json_roundtrip();
    test_json_roundtrip_tx2();
    test_postcard_roundtrip();

    // Test Tx embedded in a struct (real use case)
    test_tx_in_struct();

    // NEW: Test Poke + take() with ReceiverSlot wrapper
    test_poke_take_receiver();

    println!("\n===========================================");
    println!("Experiment complete!");
    println!("===========================================");
}
