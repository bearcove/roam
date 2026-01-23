//! roam-next: Prototype for reflection-based dispatch architecture
//!
//! # Goals
//!
//! Minimize monomorphization by doing as much as possible via reflection:
//! - Deserialize using Shape + pointer (not generic `from_slice::<T>`)
//! - Patch channel IDs via Poke
//! - Bind streams via Poke
//! - Middleware intercept via Peek (can inspect args without knowing type)
//! - Serialize response via Shape + pointer
//!
//! # Generated code should be minimal
//!
//! ```ignore
//! // What the macro generates - just type info + handler call
//! match method_id {
//!     0xABC => {
//!         let mut args = MaybeUninit::<(String, i32)>::uninit();
//!
//!         // Phase 1: Non-generic - deserialize + middleware via reflection
//!         unsafe {
//!             runtime.prepare(
//!                 args.as_mut_ptr().cast::<()>(),
//!                 <(String, i32)>::SHAPE,
//!                 payload,
//!                 ctx,
//!             ).await?;
//!         }
//!
//!         // Phase 2: Monomorphized - read args and call handler
//!         let (name, age) = unsafe { args.assume_init_read() };
//!         self.handler.create_user(name, age).await
//!     }
//! }
//! ```

use std::future::Future;
use std::pin::Pin;

use facet_core::{PtrUninit, Shape};
use facet_format::FormatDeserializer;
use facet_postcard::PostcardParser;
use facet_reflect::Partial;

/// Type-erased pointer to args or result.
pub type ErasedPtr = *mut ();

/// Middleware that can inspect requests via reflection.
///
/// Middleware sees:
/// - Context (metadata, extensions, conn_id, etc.)
/// - Args via Peek (reflection - no type knowledge needed)
///
/// Middleware does NOT know the concrete types. It uses reflection to inspect.
pub trait Middleware: Send + Sync {
    /// Intercept a request before the handler runs.
    ///
    /// - `ctx`: Request context with metadata, extensions
    /// - `args`: Peek view of deserialized args (can inspect via reflection)
    ///
    /// Return `Ok(())` to continue, `Err(rejection)` to reject.
    fn intercept(
        &self,
        ctx: &mut Context,
        args: facet::Peek<'_, 'static>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + '_>>;
}

/// Request context - what middleware and handlers see.
#[derive(Debug)]
pub struct Context {
    pub conn_id: u64,
    pub request_id: u64,
    pub method_id: u64,
    pub metadata: Vec<(String, String)>, // simplified for prototype
    pub extensions: Extensions,
}

/// Type-safe extension storage (same pattern as http crate).
#[derive(Debug, Default)]
pub struct Extensions {
    // Simplified for prototype - real impl uses TypeId-keyed HashMap
    _private: (),
}

impl Extensions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, _val: T) {
        // TODO: implement properly
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        // TODO: implement properly
        None
    }
}

/// Reason for rejecting a request.
#[derive(Debug)]
pub struct Rejection {
    pub message: String,
}

/// The runtime that handles dispatch via reflection.
///
/// This is what the generated code calls into. It handles:
/// - Deserialization (via Shape)
/// - Channel binding (via Poke)
/// - Middleware (via Peek)
/// - Calling the handler
/// - Serialization (via Shape)
pub struct DispatchRuntime {
    middleware: Vec<Box<dyn Middleware>>,
}

impl DispatchRuntime {
    pub fn new() -> Self {
        Self {
            middleware: Vec::new(),
        }
    }

    pub fn with_middleware<M: Middleware + 'static>(mut self, m: M) -> Self {
        self.middleware.push(Box::new(m));
        self
    }

    /// Prepare a request for dispatch.
    ///
    /// This does all the non-generic work:
    /// - Deserializes payload into args_ptr
    /// - Patches channel IDs (TODO)
    /// - Binds streams (TODO)
    /// - Runs middleware
    ///
    /// After this returns `Ok(())`, the caller can safely read from args_ptr
    /// and call the handler. The caller is responsible for reading the args
    /// (which moves them out) - failing to do so will leak memory.
    ///
    /// # Safety
    ///
    /// - `args_ptr` must point to valid, aligned, properly-sized uninitialized memory
    /// - On success, caller MUST read from args_ptr (to take ownership of initialized value)
    pub async unsafe fn prepare(
        &self,
        args_ptr: ErasedPtr,
        args_shape: &'static Shape,
        payload: &[u8],
        ctx: &mut Context,
    ) -> Result<(), DispatchError> {
        // 1. Deserialize into args_ptr using reflection
        self.deserialize_into(args_ptr, args_shape, payload)?;

        // 2. Patch channel IDs (TODO)
        // self.patch_channels(args_ptr, args_shape, &ctx.channels);

        // 3. Bind streams (TODO)
        // self.bind_streams(args_ptr, args_shape, registry);

        // 4. Run middleware with Peek access to args
        // SAFETY: args_ptr was just initialized by deserialize_into, args_shape matches
        let peek = unsafe {
            facet::Peek::unchecked_new(facet_core::PtrConst::new(args_ptr.cast::<u8>()), args_shape)
        };
        for mw in &self.middleware {
            mw.intercept(ctx, peek)
                .await
                .map_err(DispatchError::Rejected)?;
        }

        Ok(())
    }

    /// Deserialize payload into a type-erased pointer using Shape.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to valid, properly aligned memory for the type described by `shape`
    /// - The memory must have at least `shape.layout.size()` bytes available
    fn deserialize_into(
        &self,
        ptr: ErasedPtr,
        shape: &'static Shape,
        payload: &[u8],
    ) -> Result<(), DispatchError> {
        // Create a Partial that writes directly into caller-provided memory.
        // This avoids heap allocation - the value is constructed in-place.
        let ptr_uninit = PtrUninit::new(ptr.cast::<u8>());

        // SAFETY: Caller guarantees ptr is valid, aligned, and properly sized
        let partial: Partial<'_, false> = unsafe { Partial::from_raw(ptr_uninit, shape) }
            .map_err(|e| DispatchError::Deserialize(e.to_string()))?;

        // Use facet-format's FormatDeserializer with PostcardParser to deserialize.
        // This is non-generic - it uses the Shape for all type information.
        let parser = PostcardParser::new(payload);
        let mut deserializer: FormatDeserializer<'_, false, _> =
            FormatDeserializer::new_owned(parser);
        let partial = deserializer
            .deserialize_into(partial)
            .map_err(|e| DispatchError::Deserialize(e.to_string()))?;

        // Validate the value is fully initialized and leave it in place.
        // After this succeeds, the caller can safely read from ptr.
        partial
            .finish_in_place()
            .map_err(|e| DispatchError::Deserialize(e.to_string()))?;

        Ok(())
    }
}

impl Default for DispatchRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Deserialize(String),
    Rejected(Rejection),
}

// ============================================================================
// Example: How generated code would look
// ============================================================================

/// Example of what the macro would generate.
///
/// Note: This is just to show the pattern. Real codegen would be in roam-macros.
#[cfg(test)]
#[allow(dead_code)]
mod example_generated {
    use std::mem::MaybeUninit;

    use facet::Facet;

    use super::*;

    // User's service trait
    trait MyService {
        fn create_user(
            &self,
            name: String,
            age: i32,
        ) -> impl std::future::Future<Output = Result<User, CreateUserError>> + Send;
    }

    #[derive(Debug, Facet)]
    struct User {
        id: u64,
        name: String,
    }

    #[derive(Debug, Facet)]
    struct CreateUserError {
        message: String,
    }

    // What the macro generates:
    struct MyServiceDispatcher<H> {
        handler: H,
        runtime: DispatchRuntime,
    }

    impl<H: MyService + Send + Sync> MyServiceDispatcher<H> {
        const CREATE_USER_METHOD_ID: u64 = 0xABC123;

        async fn dispatch(
            &self,
            method_id: u64,
            payload: &[u8],
            ctx: &mut Context,
        ) -> Result<(), DispatchError> {
            match method_id {
                Self::CREATE_USER_METHOD_ID => {
                    // Allocate space for args on the stack
                    let mut args = MaybeUninit::<(String, i32)>::uninit();

                    // Phase 1: Non-generic - deserialize + middleware via reflection
                    // SAFETY: args points to valid, aligned memory for (String, i32)
                    unsafe {
                        self.runtime
                            .prepare(
                                args.as_mut_ptr().cast::<()>(),
                                <(String, i32)>::SHAPE,
                                payload,
                                ctx,
                            )
                            .await?;
                    }

                    // Phase 2: Monomorphized - read args and call handler directly
                    // SAFETY: prepare() succeeded, so args is initialized
                    let (name, age) = unsafe { args.assume_init_read() };
                    let result = self.handler.create_user(name, age).await;

                    // TODO: serialize result and send response
                    let _ = result;
                }
                _ => {
                    // Unknown method
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::MaybeUninit;

    use facet::Facet;

    use super::*;

    #[test]
    fn test_runtime_creation() {
        let _runtime = DispatchRuntime::new();
    }

    #[derive(Debug, PartialEq, Facet)]
    struct TestArgs {
        name: String,
        age: i32,
    }

    #[test]
    fn test_deserialize_into_stack() {
        let runtime = DispatchRuntime::new();

        // Create test data and serialize it
        let original = TestArgs {
            name: "Alice".to_string(),
            age: 30,
        };
        let payload = facet_postcard::to_vec(&original).expect("serialize");

        // Deserialize into stack memory
        let mut slot = MaybeUninit::<TestArgs>::uninit();
        runtime
            .deserialize_into(slot.as_mut_ptr().cast::<()>(), TestArgs::SHAPE, &payload)
            .expect("deserialize");

        // Verify the value
        let deserialized = unsafe { slot.assume_init() };
        assert_eq!(deserialized.name, "Alice");
        assert_eq!(deserialized.age, 30);
    }

    #[derive(Debug, PartialEq, Facet)]
    struct NestedArgs {
        outer_name: String,
        inner: InnerArgs,
    }

    #[derive(Debug, PartialEq, Facet)]
    struct InnerArgs {
        value: i64,
        flag: bool,
    }

    #[test]
    fn test_deserialize_nested_into_stack() {
        let runtime = DispatchRuntime::new();

        let original = NestedArgs {
            outer_name: "Test".to_string(),
            inner: InnerArgs {
                value: 42,
                flag: true,
            },
        };
        let payload = facet_postcard::to_vec(&original).expect("serialize");

        let mut slot = MaybeUninit::<NestedArgs>::uninit();
        runtime
            .deserialize_into(slot.as_mut_ptr().cast::<()>(), NestedArgs::SHAPE, &payload)
            .expect("deserialize");

        let deserialized = unsafe { slot.assume_init() };
        assert_eq!(deserialized.outer_name, "Test");
        assert_eq!(deserialized.inner.value, 42);
        assert!(deserialized.inner.flag);
    }

    #[test]
    fn test_deserialize_tuple_into_stack() {
        let runtime = DispatchRuntime::new();

        // Test with a tuple (common for RPC args)
        let original: (String, i32, bool) = ("hello".to_string(), 123, true);
        let payload = facet_postcard::to_vec(&original).expect("serialize");

        let mut slot = MaybeUninit::<(String, i32, bool)>::uninit();
        runtime
            .deserialize_into(
                slot.as_mut_ptr().cast::<()>(),
                <(String, i32, bool)>::SHAPE,
                &payload,
            )
            .expect("deserialize");

        let deserialized = unsafe { slot.assume_init() };
        assert_eq!(deserialized.0, "hello");
        assert_eq!(deserialized.1, 123);
        assert!(deserialized.2);
    }
}
