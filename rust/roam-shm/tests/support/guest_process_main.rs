//! Test guest process that can be spawned for integration tests.
//!
//! This binary is spawned by integration tests to verify multi-process
//! communication over SHM. It:
//! - Parses spawn arguments from command line
//! - Attaches to the SHM segment using the provided doorbell fd
//! - Sets up a guest driver with a test service
//! - Handles RPC calls from the host
//!
//! Usage: spawned via SpawnTicket::spawn(), receives args automatically

use facet::Facet;
use once_cell::sync::Lazy;
use roam_core::{
    ChannelRegistry, Context, MethodDescriptor, RpcPlan, Rx, ServiceDispatcher, Tx, dispatch_call,
};
use roam_shm::driver::establish_guest;
use roam_shm::spawn::{SpawnArgs, die_with_parent};
use roam_shm::transport::ShmGuestTransport;
use roam_types::{MethodId, Payload};
use std::pin::Pin;

// ============================================================================
// RPC Plans
// ============================================================================

static STRING_ARGS_PLAN: Lazy<RpcPlan> = Lazy::new(RpcPlan::for_type::<String, (), ()>);
static STRING_RESPONSE_PLAN: Lazy<&'static RpcPlan> =
    Lazy::new(|| Box::leak(Box::new(RpcPlan::for_type::<String, (), ()>())));

static I32_I32_ARGS_PLAN: Lazy<RpcPlan> = Lazy::new(RpcPlan::for_type::<(i32, i32), (), ()>);
static I32_RESPONSE_PLAN: Lazy<&'static RpcPlan> =
    Lazy::new(|| Box::leak(Box::new(RpcPlan::for_type::<i32, (), ()>())));

static RX_I32_ARGS_PLAN: Lazy<RpcPlan> = Lazy::new(RpcPlan::for_type::<Rx<i32>, Tx<i32>, Rx<i32>>);
static I64_RESPONSE_PLAN: Lazy<&'static RpcPlan> =
    Lazy::new(|| Box::leak(Box::new(RpcPlan::for_type::<i64, (), ()>())));

static U32_TX_I32_ARGS_PLAN: Lazy<RpcPlan> =
    Lazy::new(RpcPlan::for_type::<(u32, Tx<i32>), Tx<i32>, Rx<i32>>);
static UNIT_RESPONSE_PLAN: Lazy<&'static RpcPlan> =
    Lazy::new(|| Box::leak(Box::new(RpcPlan::for_type::<(), (), ()>())));

const METHOD_ECHO: MethodId = MethodId(1);
const METHOD_ADD: MethodId = MethodId(2);
const METHOD_SUM: MethodId = MethodId(3);
const METHOD_GENERATE: MethodId = MethodId(4);

static ECHO_DESC: Lazy<&'static MethodDescriptor> = Lazy::new(|| {
    Box::leak(Box::new(MethodDescriptor {
        id: METHOD_ECHO,
        service_name: "Test",
        method_name: "echo",
        args: &[],
        return_shape: <String as Facet>::SHAPE,
        args_plan: &STRING_ARGS_PLAN,
        ok_plan: *STRING_RESPONSE_PLAN,
        err_plan: *UNIT_RESPONSE_PLAN,
        doc: None,
    }))
});

static ADD_DESC: Lazy<&'static MethodDescriptor> = Lazy::new(|| {
    Box::leak(Box::new(MethodDescriptor {
        id: METHOD_ADD,
        service_name: "Test",
        method_name: "add",
        args: &[],
        return_shape: <i32 as Facet>::SHAPE,
        args_plan: &I32_I32_ARGS_PLAN,
        ok_plan: *I32_RESPONSE_PLAN,
        err_plan: *UNIT_RESPONSE_PLAN,
        doc: None,
    }))
});

static SUM_DESC: Lazy<&'static MethodDescriptor> = Lazy::new(|| {
    Box::leak(Box::new(MethodDescriptor {
        id: METHOD_SUM,
        service_name: "Test",
        method_name: "sum",
        args: &[],
        return_shape: <i64 as Facet>::SHAPE,
        args_plan: &RX_I32_ARGS_PLAN,
        ok_plan: *I64_RESPONSE_PLAN,
        err_plan: *UNIT_RESPONSE_PLAN,
        doc: None,
    }))
});

static GENERATE_DESC: Lazy<&'static MethodDescriptor> = Lazy::new(|| {
    Box::leak(Box::new(MethodDescriptor {
        id: METHOD_GENERATE,
        service_name: "Test",
        method_name: "generate",
        args: &[],
        return_shape: <() as Facet>::SHAPE,
        args_plan: &U32_TX_I32_ARGS_PLAN,
        ok_plan: *UNIT_RESPONSE_PLAN,
        err_plan: *UNIT_RESPONSE_PLAN,
        doc: None,
    }))
});

/// Test service matching the one in driver.rs tests
#[derive(Clone)]
struct TestService;

impl ServiceDispatcher for TestService {
    fn service_descriptor(&self) -> &'static roam_core::ServiceDescriptor {
        &roam_types::ServiceDescriptor::EMPTY
    }

    fn dispatch(
        &self,
        cx: Context,
        payload: Vec<u8>,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        match cx.method_id().0 {
            // Echo method: returns the input unchanged
            1 => dispatch_call::<String, String, (), _, _>(
                &cx,
                Payload(payload),
                registry,
                *ECHO_DESC,
                |input: String| async move { Ok(input) },
            ),
            // Add method: adds two numbers
            2 => dispatch_call::<(i32, i32), i32, (), _, _>(
                &cx,
                Payload(payload),
                registry,
                *ADD_DESC,
                |(a, b): (i32, i32)| async move { Ok(a + b) },
            ),
            // Sum method: client streams numbers, server returns sum
            3 => dispatch_call::<Rx<i32>, i64, (), _, _>(
                &cx,
                Payload(payload),
                registry,
                *SUM_DESC,
                |mut input: Rx<i32>| async move {
                    let mut sum: i64 = 0;
                    while let Ok(Some(value)) = input.recv().await {
                        sum += value as i64;
                    }
                    Ok(sum)
                },
            ),
            // Generate method: server streams numbers back to client
            4 => dispatch_call::<(u32, Tx<i32>), (), (), _, _>(
                &cx,
                Payload(payload),
                registry,
                *GENERATE_DESC,
                |(count, output): (u32, Tx<i32>)| async move {
                    for i in 0..count {
                        output.send(&(i as i32)).await.ok();
                    }
                    Ok(())
                },
            ),
            _ => roam_core::dispatch_unknown_method(&cx, registry),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Die if parent dies (even via SIGKILL)
    die_with_parent();

    // Parse spawn arguments from command line
    let args = SpawnArgs::from_env().expect("failed to parse spawn args");

    // Create guest transport from spawn args (includes doorbell setup)
    let transport =
        ShmGuestTransport::from_spawn_args(args).expect("failed to create guest transport");
    let (_handle, _incoming_connections, driver) = establish_guest(transport, TestService);

    // Run the driver until the host disconnects
    driver.run().await.expect("guest driver failed");
}
