use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use message::{Message, RequestMessage, TestMessage, UnsignedMessage};
use signed;
use tcp::Network;

struct ZenoState {
    pubkeys: Vec<signed::Public>,
}

#[derive(Clone)]
pub struct Zeno {
    me: signed::Public,
    state: Arc<Mutex<ZenoState>>,
}

fn on_request_message(z: Zeno, _: RequestMessage, _: Network) -> Message {
    return Message::Unsigned(UnsignedMessage::Test(TestMessage { c: z.me }));
}

impl Zeno {
    fn match_unsigned_message(z: Zeno, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => Some(on_request_message(z, rm, n)),
            _ => None,
        }
    }

    fn handle_message(z: Zeno, m: Message, n: Network) -> Option<Message> {
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
) -> Zeno {
    let zeno = Zeno {
        me: pubkey,
        state: Arc::new(Mutex::new(ZenoState {
            pubkeys: pubkeys_to_url.keys().map(|p| p.clone()).collect(),
        })),
    };
    Network::new(
        url,
        pubkeys_to_url,
        Some((Zeno::handle_message, zeno.clone())),
    );
    zeno
}

#[cfg(test)]
mod tests {
    use super::start_zeno;
    use signed;
    use std::collections::HashMap;
    #[test]
    fn basic() {
        let urls = vec!["127.0.0.1:44444".to_string(),
                        "127.0.0.1:55555".to_string()];
        let mut pubkeys_to_urls = HashMap::new();
        let mut pubkeys = Vec::new();
        let mut zenos = Vec::new();
        for i in 0..2 {
            let pubkey = signed::gen_keys().0;
            println!("URL IS: {:?}", urls[i]);
            pubkeys.push(pubkey.clone());
            pubkeys_to_urls.insert(pubkey, urls[i].clone());
        }
        for i in 0..2 {
            zenos.push(
                start_zeno(urls[i].clone(), pubkeys[i],
                          pubkeys_to_urls.clone()));
        }
    }
}
