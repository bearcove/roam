#![cfg(not(target_arch = "wasm32"))]
//! Channel binding infrastructure for connecting Tx/Rx handles to the driver.
//!
//! Four binding functions handle the four combinations of caller/callee × args/return:
//!
//! - [`bind_channels_caller_args`]: Caller-side, arg position. Allocates IDs,
//!   stores bindings in the shared core so the paired handle can use them.
//! - [`bind_channels_callee_args`]: Callee-side, arg position. Binds deserialized
//!   standalone handles directly using IDs from `Request.channels`.
//! - [`bind_channels_callee_return`]: Callee-side, return position. Allocates IDs,
//!   stores bindings in the shared core so the paired handle can use them.
//! - [`bind_channels_caller_return`]: Caller-side, return position. Binds deserialized
//!   standalone handles directly using IDs from `Response.channels`.

use std::sync::Arc;

use facet_core::PtrMut;
use facet_path::PathAccessError;
use tokio::sync::mpsc;

use crate::ChannelId;
use crate::channel::{
    ChannelBinding, ChannelSink, CoreSlot, IncomingChannelMessage, ReceiverSlot, SinkSlot,
};
use crate::rpc_plan::{ChannelKind, RpcPlan};

/// Trait for channel operations, implemented by the session driver.
///
/// This abstraction lets the binding functions and macro-generated code bind
/// channels without depending on concrete driver types.
pub trait ChannelBinder: Send + Sync {
    /// Allocate a channel ID and create a sink for sending items.
    ///
    /// `initial_credit` is the const generic `N` from `Tx<T, N>` or `Rx<T, N>`.
    fn create_tx(&self, initial_credit: u32) -> (ChannelId, Arc<dyn ChannelSink>);

    /// Allocate a channel ID, register it for routing, and return a receiver.
    fn create_rx(&self) -> (ChannelId, mpsc::Receiver<IncomingChannelMessage>);

    /// Create a sink for a known channel ID (callee side).
    ///
    /// The channel ID comes from `Request.channels` or `Response.channels`.
    /// `initial_credit` is the const generic `N` from `Tx<T, N>`.
    fn bind_tx(&self, channel_id: ChannelId, initial_credit: u32) -> Arc<dyn ChannelSink>;

    /// Register an inbound channel by ID and return the receiver (callee side).
    ///
    /// The channel ID comes from `Request.channels` or `Response.channels`.
    fn register_rx(&self, channel_id: ChannelId) -> mpsc::Receiver<IncomingChannelMessage>;
}

// r[impl rpc.channel.binding.caller-args]
// r[impl rpc.channel.allocation]
/// Bind channels in args on the **caller** side, returning channel IDs.
///
/// The caller created `(tx, rx)` pairs via `channel()`. Only one handle from
/// each pair is in the args; the other was kept by the caller. This function
/// stores bindings in the shared core so the kept handle can use them.
///
/// # Safety
///
/// `args_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_caller_args(
    args_ptr: *mut u8,
    plan: &RpcPlan,
    binder: &dyn ChannelBinder,
) -> Vec<ChannelId> {
    let shape = plan.shape;
    let mut channel_ids = Vec::new();

    for loc in plan.channel_locations {
        // SAFETY: caller guarantees args_ptr is valid and initialized for this shape
        let poke = unsafe { facet::Poke::from_raw_parts(PtrMut::new(args_ptr), shape) };

        match poke.at_path_mut(&loc.path) {
            Ok(channel_poke) => match loc.kind {
                // r[impl rpc.channel.binding.caller-args.rx]
                // Rx in args: handler receives, caller sends.
                // Create a sink and store it in the shared core so the caller's
                // paired Tx can send through it.
                ChannelKind::Rx => {
                    let (channel_id, sink) = binder.create_tx(loc.initial_credit);
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut core_field) = ps.field_by_name("core")
                        && let Ok(slot) = core_field.get_mut::<CoreSlot>()
                    {
                        if let Some(core) = &slot.inner {
                            core.set_binding(ChannelBinding::Sink(sink));
                        }
                    }
                }
                // r[impl rpc.channel.binding.caller-args.tx]
                // Tx in args: handler sends, caller receives.
                // Create a receiver and store it in the shared core so the caller's
                // paired Rx can receive from it.
                ChannelKind::Tx => {
                    let (channel_id, receiver) = binder.create_rx();
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut core_field) = ps.field_by_name("core")
                        && let Ok(slot) = core_field.get_mut::<CoreSlot>()
                    {
                        if let Some(core) = &slot.inner {
                            core.set_binding(ChannelBinding::Receiver(receiver));
                        }
                    }
                }
            },
            Err(PathAccessError::OptionIsNone { .. }) => {
                // Option<Tx/Rx> is None — skip
            }
            Err(_) => {}
        }
    }

    channel_ids
}

// r[impl rpc.channel.binding]
// r[impl rpc.channel.binding.callee-args]
/// Bind channels in deserialized args on the **callee** side.
///
/// Handles are standalone (not part of a pair). Bind directly into the
/// handle's local slot using channel IDs from `Request.channels`.
///
/// # Safety
///
/// `args_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_callee_args(
    args_ptr: *mut u8,
    plan: &RpcPlan,
    channel_ids: &[ChannelId],
    binder: &dyn ChannelBinder,
) {
    let shape = plan.shape;
    let mut id_idx = 0;

    for loc in plan.channel_locations {
        // SAFETY: caller guarantees args_ptr is valid and initialized for this shape
        let poke = unsafe { facet::Poke::from_raw_parts(PtrMut::new(args_ptr), shape) };

        match poke.at_path_mut(&loc.path) {
            Ok(channel_poke) => {
                if id_idx >= channel_ids.len() {
                    break;
                }
                let channel_id = channel_ids[id_idx];
                id_idx += 1;

                match loc.kind {
                    // r[impl rpc.channel.binding.callee-args.tx]
                    // Tx in args: handler sends. Bind a sink directly.
                    ChannelKind::Tx => {
                        let sink = binder.bind_tx(channel_id, loc.initial_credit);
                        if let Ok(mut ps) = channel_poke.into_struct()
                            && let Ok(mut sink_field) = ps.field_by_name("sink")
                            && let Ok(slot) = sink_field.get_mut::<SinkSlot>()
                        {
                            slot.inner = Some(sink);
                        }
                    }
                    // r[impl rpc.channel.binding.callee-args.rx]
                    // Rx in args: handler receives. Register and bind a receiver directly.
                    ChannelKind::Rx => {
                        let receiver = binder.register_rx(channel_id);
                        if let Ok(mut ps) = channel_poke.into_struct()
                            && let Ok(mut receiver_field) = ps.field_by_name("receiver")
                            && let Ok(slot) = receiver_field.get_mut::<ReceiverSlot>()
                        {
                            slot.inner = Some(receiver);
                        }
                    }
                }
            }
            Err(PathAccessError::OptionIsNone { .. }) => {
                // Option<Tx/Rx> is None — skip this channel location
            }
            Err(_) => {}
        }
    }
}

// r[impl rpc.channel.binding.callee-return]
/// Bind channels in a return value on the **callee** side, returning channel IDs.
///
/// The callee created `(tx, rx)` pairs via `channel()`. Only one handle from
/// each pair is in the return value; the other was kept by the callee. This
/// function stores bindings in the shared core so the kept handle can use them.
///
/// Channel IDs are collected into `Response.channels`.
///
/// # Safety
///
/// `ret_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_callee_return(
    ret_ptr: *mut u8,
    plan: &RpcPlan,
    binder: &dyn ChannelBinder,
) -> Vec<ChannelId> {
    let shape = plan.shape;
    let mut channel_ids = Vec::new();

    for loc in plan.channel_locations {
        // SAFETY: caller guarantees ret_ptr is valid and initialized for this shape
        let poke = unsafe { facet::Poke::from_raw_parts(PtrMut::new(ret_ptr), shape) };

        match poke.at_path_mut(&loc.path) {
            Ok(channel_poke) => match loc.kind {
                // r[impl rpc.channel.binding.callee-return.rx]
                // Rx in return: caller receives, callee sends.
                // Create a sink and store it in the shared core so the callee's
                // paired Tx can send through it.
                ChannelKind::Rx => {
                    let (channel_id, sink) = binder.create_tx(loc.initial_credit);
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut core_field) = ps.field_by_name("core")
                        && let Ok(slot) = core_field.get_mut::<CoreSlot>()
                        && let Some(core) = &slot.inner
                    {
                        core.set_binding(ChannelBinding::Sink(sink));
                    }
                }
                // r[impl rpc.channel.binding.callee-return.tx]
                // Tx in return: caller sends, callee receives.
                // Create a receiver and store it in the shared core so the callee's
                // paired Rx can receive from it.
                ChannelKind::Tx => {
                    let (channel_id, receiver) = binder.create_rx();
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut core_field) = ps.field_by_name("core")
                        && let Ok(slot) = core_field.get_mut::<CoreSlot>()
                        && let Some(core) = &slot.inner
                    {
                        core.set_binding(ChannelBinding::Receiver(receiver));
                    }
                }
            },
            Err(PathAccessError::OptionIsNone { .. }) => {
                // Option<Tx/Rx> is None — skip
            }
            Err(_) => {}
        }
    }

    channel_ids
}

// r[impl rpc.channel.binding.caller-return]
/// Bind channels in a deserialized return value on the **caller** side.
///
/// Handles are standalone (deserialized from the response). Bind directly
/// into the handle's local slot using channel IDs from `Response.channels`.
///
/// # Safety
///
/// `ret_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_caller_return(
    ret_ptr: *mut u8,
    plan: &RpcPlan,
    channel_ids: &[ChannelId],
    binder: &dyn ChannelBinder,
) {
    let shape = plan.shape;
    let mut id_idx = 0;

    for loc in plan.channel_locations {
        // SAFETY: caller guarantees ret_ptr is valid and initialized for this shape
        let poke = unsafe { facet::Poke::from_raw_parts(PtrMut::new(ret_ptr), shape) };

        match poke.at_path_mut(&loc.path) {
            Ok(channel_poke) => {
                if id_idx >= channel_ids.len() {
                    break;
                }
                let channel_id = channel_ids[id_idx];
                id_idx += 1;

                match loc.kind {
                    // r[impl rpc.channel.binding.caller-return.tx]
                    // Tx in return: caller sends. Bind a sink directly.
                    ChannelKind::Tx => {
                        let sink = binder.bind_tx(channel_id, loc.initial_credit);
                        if let Ok(mut ps) = channel_poke.into_struct()
                            && let Ok(mut sink_field) = ps.field_by_name("sink")
                            && let Ok(slot) = sink_field.get_mut::<SinkSlot>()
                        {
                            slot.inner = Some(sink);
                        }
                    }
                    // r[impl rpc.channel.binding.caller-return.rx]
                    // Rx in return: caller receives. Register and bind a receiver directly.
                    ChannelKind::Rx => {
                        let receiver = binder.register_rx(channel_id);
                        if let Ok(mut ps) = channel_poke.into_struct()
                            && let Ok(mut receiver_field) = ps.field_by_name("receiver")
                            && let Ok(slot) = receiver_field.get_mut::<ReceiverSlot>()
                        {
                            slot.inner = Some(receiver);
                        }
                    }
                }
            }
            Err(PathAccessError::OptionIsNone { .. }) => {
                // Option<Tx/Rx> is None — skip
            }
            Err(_) => {}
        }
    }
}
