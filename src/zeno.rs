use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use message::{Message, RequestMessage, TestMessage, UnsignedMessage};
use signed;
use tcp::Network;

struct ZenoState {}

#[derive(Clone)]
pub struct Zeno {
    me: signed::Public,
    pubkeys: Vec<signed::Public>,
    state: Arc<Mutex<ZenoState>>,
}

fn on_request_message(z: Arc<Zeno>, _: RequestMessage, _: Network) -> Message {
    return Message::Unsigned(UnsignedMessage::Test(TestMessage { c: z.me }));
}

impl Zeno {
    fn match_unsigned_message(z: Arc<Zeno>, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => Some(on_request_message(z, rm, n)),
            _ => None,
        }
    }

    fn handle_message(z: Arc<Zeno>, m: Message, n: Network) -> Option<Message> {
        match m {
            Message::Unsigned(um) => Zeno::match_unsigned_message(z, um, n),
            Message::Signed(sm) => Zeno::match_unsigned_message(z, sm.base, n),
        }
    }
}

pub fn start_zeno(
    url: String,
    pubkey: signed::Public,
    pubkeys_to_url: HashMap<signed::Public, String>,
) -> Arc<Zeno> {
    let zeno = Arc::new(Zeno {
        me: pubkey,
        pubkeys: pubkeys_to_url.keys().map(|p| p.clone()).collect(),
        state: Arc::new(Mutex::new(ZenoState {})),
    });
    Network::new(
        url,
        pubkeys_to_url,
        Some((Zeno::handle_message, zeno.clone())),
    );
    zeno
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic() {}
}
