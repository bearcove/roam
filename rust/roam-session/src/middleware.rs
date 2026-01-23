//! Middleware for intercepting requests before dispatch.
//!
//! Middleware wraps a [`ServiceDispatcher`] and runs before the inner dispatcher,
//! seeing the request context (including metadata). It can:
//!
//! - Reject requests (e.g., authentication failure)
//! - Add values to `Context::extensions` for handlers to retrieve
//! - Log, trace, or meter requests
//!
//! Middleware does NOT see typed request/response payloads — that's the
//! dispatcher's job. This keeps middleware simple and composable.
//!
//! # Example
//!
//! ```ignore
//! use roam_session::{Middleware, WithMiddleware, Context, Rejection};
//! use std::sync::Arc;
//!
//! struct AuthMiddleware { /* ... */ }
//!
//! impl Middleware for AuthMiddleware {
//!     fn intercept<'a>(
//!         &'a self,
//!         ctx: &'a mut Context,
//!     ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>> {
//!         Box::pin(async move {
//!             // Check for auth token in metadata
//!             let token = ctx.metadata.iter()
//!                 .find(|(k, _)| k == "auth-token")
//!                 .and_then(|(_, v)| v.as_string());
//!
//!             let Some(token) = token else {
//!                 return Err(Rejection::unauthenticated("missing auth-token"));
//!             };
//!
//!             // Validate token (async database lookup, etc.)
//!             let user = validate_token(token).await?;
//!
//!             // Store user in extensions for handler access
//!             ctx.extensions.insert(user);
//!
//!             Ok(())
//!         })
//!     }
//! }
//!
//! // Wrap a dispatcher with middleware
//! let dispatcher = WithMiddleware::new(
//!     Arc::new(AuthMiddleware::new()),
//!     MyServiceDispatcher::new(handler),
//! );
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::{ChannelRegistry, Context, DriverMessage, ServiceDispatcher};

/// Reason for rejecting a request.
///
/// When middleware rejects a request, this is sent back as the response.
#[derive(Debug, Clone)]
pub struct Rejection {
    /// Error code for programmatic handling.
    pub code: RejectionCode,
    /// Human-readable message.
    pub message: String,
}

/// Standard rejection codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectionCode {
    /// Request lacks required authentication.
    Unauthenticated,
    /// Caller is authenticated but not authorized for this operation.
    PermissionDenied,
    /// Rate limit exceeded.
    RateLimited,
    /// Request is invalid (bad metadata, etc.).
    InvalidRequest,
    /// Internal middleware error.
    Internal,
}

impl Rejection {
    /// Create an "unauthenticated" rejection.
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self {
            code: RejectionCode::Unauthenticated,
            message: message.into(),
        }
    }

    /// Create a "permission denied" rejection.
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self {
            code: RejectionCode::PermissionDenied,
            message: message.into(),
        }
    }

    /// Create a "rate limited" rejection.
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self {
            code: RejectionCode::RateLimited,
            message: message.into(),
        }
    }

    /// Create an "invalid request" rejection.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: RejectionCode::InvalidRequest,
            message: message.into(),
        }
    }

    /// Create an "internal" rejection.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: RejectionCode::Internal,
            message: message.into(),
        }
    }
}

/// Middleware that can intercept requests before dispatch.
///
/// Middleware sees the request context (including metadata) but not the
/// typed payload. It can:
/// - Reject the request by returning `Err(Rejection)`
/// - Continue by returning `Ok(())`
/// - Add values to `ctx.extensions` for handlers
///
/// Middleware is async to support operations like database lookups for
/// token validation.
pub trait Middleware: Send + Sync {
    /// Intercept a request before dispatch.
    ///
    /// The context contains:
    /// - `conn_id`: Which connection this request came from
    /// - `method_id`: The method being called (as u64 hash)
    /// - `metadata`: Key-value pairs from the request
    /// - `extensions`: Where middleware can store values for handlers
    ///
    /// Return `Ok(())` to continue to the dispatcher.
    /// Return `Err(rejection)` to reject the request.
    fn intercept<'a>(
        &'a self,
        ctx: &'a mut Context,
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>>;
}

/// A dispatcher that runs middleware before delegating to an inner dispatcher.
///
/// # Type Parameters
///
/// - `D`: The inner dispatcher type
///
/// The middleware is stored as `Arc<dyn Middleware>` to allow sharing and
/// to satisfy the `'static` bound on the returned future.
pub struct WithMiddleware<D> {
    middleware: Arc<dyn Middleware>,
    inner: D,
}

impl<D> WithMiddleware<D>
where
    D: ServiceDispatcher,
{
    /// Create a new middleware-wrapped dispatcher.
    pub fn new(middleware: Arc<dyn Middleware>, inner: D) -> Self {
        Self { middleware, inner }
    }
}

impl<D: Clone> Clone for WithMiddleware<D> {
    fn clone(&self) -> Self {
        Self {
            middleware: self.middleware.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<D> ServiceDispatcher for WithMiddleware<D>
where
    D: ServiceDispatcher + Clone + 'static,
{
    fn method_ids(&self) -> Vec<u64> {
        self.inner.method_ids()
    }

    fn dispatch(
        &self,
        mut cx: Context,
        payload: Vec<u8>,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // Capture what we need for the async block
        let driver_tx = registry.driver_tx();
        let conn_id = cx.conn_id;
        let request_id = cx.request_id.raw();

        // Clone middleware (Arc clone is cheap)
        let middleware = self.middleware.clone();

        // Clone inner dispatcher so we can call it in the async block
        let inner = self.inner.clone();

        // Do stream binding NOW, before the async block.
        // If middleware rejects, these get cleaned up via drops (same as handler errors).
        //
        // We need to prepare everything the inner dispatcher needs, then call it
        // inside the async block after middleware passes.
        //
        // The tricky part: inner.dispatch() needs &mut registry, but we can't
        // hold that across the await. However, the inner dispatcher will also
        // just extract what it needs and return a 'static future.
        //
        // Solution: We prepare a "dispatch continuation" that captures everything
        // needed. For generated dispatchers, this means doing the bind_streams
        // work here. For ForwardingDispatcher, it extracts what it needs.
        //
        // Actually, the simplest approach: call inner.dispatch() NOW to get
        // its future, then conditionally run it after middleware.

        let inner_future = inner.dispatch(cx.clone(), payload, registry);

        Box::pin(async move {
            // Run middleware (async)
            match middleware.intercept(&mut cx).await {
                Ok(()) => {
                    // Middleware passed, run the inner dispatch
                    inner_future.await
                }
                Err(_rejection) => {
                    // Middleware rejected, send error response
                    warn!(
                        code = ?_rejection.code,
                        message = %_rejection.message,
                        "middleware rejected request"
                    );
                    let _ = driver_tx
                        .send(DriverMessage::Response {
                            conn_id,
                            request_id,
                            channels: Vec::new(),
                            // Result::Err(1) + RoamError::InvalidPayload(2)
                            // TODO: Add a proper rejection error variant to RoamError
                            payload: vec![1, 2],
                        })
                        .await;
                }
            }
        })
    }
}

/// Middleware that does nothing (passes all requests through).
///
/// Useful as a default or for testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMiddleware;

impl Middleware for NoopMiddleware {
    fn intercept<'a>(
        &'a self,
        _ctx: &'a mut Context,
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

/// Compose multiple middleware into a single middleware.
///
/// Middleware runs in order: first middleware added runs first.
pub struct MiddlewareStack {
    layers: Vec<Arc<dyn Middleware>>,
}

impl MiddlewareStack {
    /// Create a new empty middleware stack.
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Add middleware to the stack.
    ///
    /// Middleware runs in the order added.
    pub fn with<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.layers.push(Arc::new(middleware));
        self
    }

    /// Add an already-Arc'd middleware to the stack.
    pub fn with_arc(mut self, middleware: Arc<dyn Middleware>) -> Self {
        self.layers.push(middleware);
        self
    }
}

impl Default for MiddlewareStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for MiddlewareStack {
    fn intercept<'a>(
        &'a self,
        ctx: &'a mut Context,
    ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>> {
        Box::pin(async move {
            for layer in &self.layers {
                layer.intercept(ctx).await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestMiddleware {
        should_reject: bool,
    }

    impl Middleware for TestMiddleware {
        fn intercept<'a>(
            &'a self,
            ctx: &'a mut Context,
        ) -> Pin<Box<dyn Future<Output = Result<(), Rejection>> + Send + 'a>> {
            let should_reject = self.should_reject;
            Box::pin(async move {
                if should_reject {
                    Err(Rejection::unauthenticated("test rejection"))
                } else {
                    ctx.extensions.insert(42i32);
                    Ok(())
                }
            })
        }
    }

    #[test]
    fn test_middleware_stack() {
        // Just test that it compiles and types work
        let stack = MiddlewareStack::new()
            .with(NoopMiddleware)
            .with(TestMiddleware {
                should_reject: false,
            });

        assert_eq!(stack.layers.len(), 2);
    }
}
