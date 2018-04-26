use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use message::Message;
use signed;
use tcp::Network;

struct ZenoState {}

#[derive(Clone)]
pub struct Zeno {
    me: signed::Public,
    pubkeys: Vec<signed::Public>,
    state: Arc<Mutex<ZenoState>>,
}

impl Zeno {
    fn handle_message(_: Arc<Zeno>, _: Message, _: Network) -> Option<Message> {
        None
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
    Network::new(url, pubkeys_to_url, (Zeno::handle_message, zeno.clone()));
    zeno
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic() {}
}
