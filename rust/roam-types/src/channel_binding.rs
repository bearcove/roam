//! Channel binding infrastructure for connecting Tx/Rx handles to the driver.
//!
//! The [`ChannelBinder`] trait is implemented by the session driver and provides
//! the operations needed to bind channels: creating sinks for outbound Tx handles,
//! and registering inbound Rx handles with the driver's channel routing table.
//!
//! The [`bind_channels_server`] and [`bind_channels_client`] functions use facet
//! reflection (Poke) to navigate to Tx/Rx fields within a deserialized args struct
//! and bind them, without requiring the concrete type to be known.

use std::sync::Arc;

use facet_core::PtrMut;
use facet_path::PathAccessError;
use tokio::sync::mpsc;

use crate::ChannelId;
use crate::channel::{ChannelSink, IncomingChannelMessage, ReceiverSlot, SinkSlot};
use crate::rpc_plan::{ChannelKind, RpcPlan};

/// Trait for channel operations, implemented by the session driver.
///
/// This abstraction lets the macro-generated dispatcher and client code bind
/// channels without depending on concrete driver types.
pub trait ChannelBinder: Send + Sync {
    /// Create an outbound channel (Tx on our side).
    ///
    /// Allocates a new channel ID and returns a sink for sending items.
    /// `initial_credit` is the const generic `N` from `Tx<T, N>`.
    /// Used by the **client** when building `RequestCall.channels`.
    fn create_tx(&self, initial_credit: u32) -> (ChannelId, Arc<dyn ChannelSink>);

    /// Create an inbound channel (Rx on our side).
    ///
    /// Allocates a new channel ID, registers it for routing, and returns
    /// a receiver. Used by the **client** when building `RequestCall.channels`.
    fn create_rx(&self) -> (ChannelId, mpsc::Receiver<IncomingChannelMessage>);

    /// Create a sink for a known channel ID (server side).
    ///
    /// The channel ID comes from `RequestCall.channels`. The server creates
    /// a sink that sends items to the client on this channel.
    /// `initial_credit` is the const generic `N` from `Tx<T, N>`.
    fn bind_tx(&self, channel_id: ChannelId, initial_credit: u32) -> Arc<dyn ChannelSink>;

    /// Register an inbound channel by ID and return the receiver (server side).
    ///
    /// The channel ID comes from `RequestCall.channels`. The server registers
    /// it so the driver routes incoming items to this receiver.
    fn register_rx(&self, channel_id: ChannelId) -> mpsc::Receiver<IncomingChannelMessage>;
}

// r[impl rpc.channel.binding]
/// Bind channels in a deserialized args value on the **server** side.
///
/// Iterates the precomputed channel locations in `plan`, navigates to each
/// Tx/Rx field via `Poke::at_path_mut`, and binds it using the `binder`:
/// - Tx fields get a sink (server sends to client)
/// - Rx fields get a receiver (server receives from client)
///
/// Channel IDs come from `RequestCall.channels`, matched positionally with
/// `plan.channel_locations`.
///
/// # Safety
///
/// `args_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_server(
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
                    ChannelKind::Tx => {
                        let sink = binder.bind_tx(channel_id, loc.initial_credit);
                        if let Ok(mut ps) = channel_poke.into_struct()
                            && let Ok(mut sink_field) = ps.field_by_name("sink")
                            && let Ok(slot) = sink_field.get_mut::<SinkSlot>()
                        {
                            slot.inner = Some(sink);
                        }
                    }
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

// r[impl rpc.channel.allocation]
/// Bind channels in an args value on the **client** side, returning channel IDs.
///
/// Iterates the precomputed channel locations in `plan`, navigates to each
/// Tx/Rx field via `Poke::at_path_mut`, allocates a channel ID, binds the
/// field, and collects the IDs for `RequestCall.channels`.
///
/// - Tx fields: client allocates ID + sink (client sends to server)
/// - Rx fields: client allocates ID + receiver (client receives from server)
///
/// # Safety
///
/// `args_ptr` must point to valid, initialized memory for a value whose
/// shape matches `plan.shape`.
#[allow(unsafe_code)]
pub unsafe fn bind_channels_client(
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
                ChannelKind::Tx => {
                    let (channel_id, sink) = binder.create_tx(loc.initial_credit);
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut sink_field) = ps.field_by_name("sink")
                        && let Ok(slot) = sink_field.get_mut::<SinkSlot>()
                    {
                        slot.inner = Some(sink);
                    }
                }
                ChannelKind::Rx => {
                    let (channel_id, receiver) = binder.create_rx();
                    channel_ids.push(channel_id);
                    if let Ok(mut ps) = channel_poke.into_struct()
                        && let Ok(mut receiver_field) = ps.field_by_name("receiver")
                        && let Ok(slot) = receiver_field.get_mut::<ReceiverSlot>()
                    {
                        slot.inner = Some(receiver);
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
