use std::env;
use std::fs;

use telex_types::{Message, MessagePayload, Payload, RequestBody};

fn can_serialize_after_decode(message: &Message<'_>) -> bool {
    match &message.payload {
        MessagePayload::RequestMessage(req) => match &req.body {
            RequestBody::Call(call) => matches!(call.args, Payload::Outgoing { .. }),
            RequestBody::Response(resp) => matches!(resp.ret, Payload::Outgoing { .. }),
            RequestBody::Cancel(_) => true,
        },
        MessagePayload::ChannelMessage(ch) => match &ch.body {
            telex_types::ChannelBody::Item(item) => matches!(item.item, Payload::Outgoing { .. }),
            telex_types::ChannelBody::Close(_) => true,
            telex_types::ChannelBody::Reset(_) => true,
            telex_types::ChannelBody::GrantCredit(_) => true,
        },
        _ => true,
    }
}

fn main() {
    let mut args = env::args();
    let _exe = args.next();
    let path = args
        .next()
        .expect("usage: protocol_decode_repro <crash-file>");
    let data = fs::read(path).expect("read input file");

    let message = telex::telex_postcard::from_slice_borrowed::<telex_types::Message<'_>>(&data)
        .expect("decode should succeed for crashing input");
    if can_serialize_after_decode(&message) {
        let _encoded = telex::telex_postcard::to_vec(&message).expect("re-encode should succeed");
    }
}
