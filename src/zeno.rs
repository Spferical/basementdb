use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use digest::HashChain;
use message::{Message, RequestMessage, TestMessage, UnsignedMessage};
use signed;
use signed::Signed;
use tcp::Network;

#[allow(dead_code)]
enum ZenoStatus {
    Replica,
    Primary,
}

#[allow(dead_code)]
struct ZenoState {
    pubkeys: Vec<signed::Public>,

    // See 4.3
    n: i64,
    v: i64,
    h_n: HashChain,
    requests: HashMap<signed::Public, Vec<Signed<RequestMessage>>>,
    replies: HashMap<signed::Public, Option<UnsignedMessage>>,

    status: ZenoStatus,
}

#[derive(Clone)]
pub struct Zeno {
    me: signed::Public,
    state: Arc<Mutex<ZenoState>>,
}

fn on_request_message(z: Zeno, m: RequestMessage, n: Network) -> Message {
    let zs: &mut ZenoState = &mut *z.state.lock().unwrap();

    let last_t: i64;

    if !zs.requests.contains_key(&m.c) {
        zs.requests.insert(m.c, Vec::new());
    }

    if zs.requests[&m.c].len() > 0 {
        let last_req_option = zs.requests[&m.c].last();
        let last_req = last_req_option.unwrap();
        last_t = last_req.base.t as i64;
    } else {
        last_t = -1;
    }

    if last_t == m.t as i64 - 1 {}
    return Message::Unsigned(UnsignedMessage::Test(TestMessage { c: z.me }));
}

impl Zeno {
    fn verifier(m: Signed<UnsignedMessage>) -> Option<UnsignedMessage> {
        match m.clone().base {
            UnsignedMessage::Request(rm) => m.verify(&rm.c),
            _ => None,
        }
    }

    fn match_unsigned_message(z: Zeno, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => Some(on_request_message(z, rm, n)),
            _ => None,
        }
    }

    fn handle_message(z: Zeno, m: Message, n: Network) -> Option<Message> {
        match m {
            Message::Unsigned(um) => Zeno::match_unsigned_message(z, um, n),
            Message::Signed(sm) => match Zeno::verifier(sm) {
                None => {
                    println!("Unable to verify message!");
                    None
                }
                Some(u) => Zeno::match_unsigned_message(z, u, n),
            },
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
            pubkeys: pubkeys_to_url
                .keys()
                .filter(|&&p| p != pubkey)
                .map(|p| p.clone())
                .collect(),
            n: -1,
            v: 0,
            h_n: HashChain::new(),
            requests: HashMap::new(),
            replies: HashMap::new(),
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
        let urls = vec![
            "127.0.0.1:44441".to_string(),
            "127.0.0.1:44442".to_string(),
            "127.0.0.1:44443".to_string(),
            "127.0.0.1:44444".to_string(),
        ];
        let mut pubkeys_to_urls = HashMap::new();
        let mut pubkeys = Vec::new();
        let mut zenos = Vec::new();
        for i in 0..4 {
            let pubkey = signed::gen_keys().0;
            pubkeys.push(pubkey.clone());
            pubkeys_to_urls.insert(pubkey, urls[i].clone());
        }

        for i in 0..4 {
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
