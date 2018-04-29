use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use digest::{HashChain, HashDigest};
use message::{ClientResponseMessage, Message, RequestMessage, TestMessage, UnsignedMessage};
use signed;
use signed::Signed;
use tcp::Network;

enum ZenoStatus {
    Replica,
    Primary,
}

struct ZenoState {
    pubkeys: Vec<signed::Public>,

    // See 4.3
    n: i64,
    v: i64,
    h_n: HashChain,
    requests: HashMap<signed::Public, Vec<Signed<Message>>>,
    replies: HashMap<signed::Public, Option<Signed<Message>>>,

    status: ZenoStatus,
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
            n: -1,
            v: 0,
            h_n: HashChain::new(),
            requests: pubkeys_to_url
                .keys()
                .map(|p| (p.clone(), Vec::new()))
                .collect(),
            replies: pubkeys_to_url.keys().map(|p| (p.clone(), None)).collect(),
            status: ZenoStatus::Replica,
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
    use std::sync::mpsc;
    use std::thread;
    use std::time;
    use zeno_client;

    #[test]
    fn basic() {
        let urls = vec!["127.0.0.1:44444".to_string(), "127.0.0.1:55555".to_string()];
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
            zenos.push(start_zeno(
                urls[i].clone(),
                pubkeys[i],
                pubkeys_to_urls.clone(),
            ));
        }
        let (tx, rx) = mpsc::channel();
        let t = thread::spawn(move || {
            let mut c = zeno_client::Client::new(signed::gen_keys(), pubkeys_to_urls.clone());
            c.request(vec![], false);
            tx.send(()).unwrap();
        });
        assert_eq!(rx.recv_timeout(time::Duration::from_secs(1)), Ok(()));
        t.join().unwrap();
    }
}
