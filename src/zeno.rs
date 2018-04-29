use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use digest;
use digest::HashChain;
use message::{Message, OrderedRequestMessage, RequestMessage, TestMessage, UnsignedMessage};
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
    h: HashChain,
    requests: HashMap<signed::Public, Vec<usize>>,
    replies: HashMap<signed::Public, Option<UnsignedMessage>>,

    all_requests: Vec<RequestMessage>,

    status: ZenoStatus,
}

#[derive(Clone)]
pub struct Zeno {
    me: signed::Public,
    private_me: signed::Private,
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
        last_t = zs.all_requests[*last_req].t as i64;
    } else {
        last_t = -1;
    }

    if last_t == m.t as i64 - 1 {
        assert!(zs.all_requests.len() == (zs.n + 1) as usize);
        zs.all_requests.push(m.clone());
        zs.requests.get_mut(&m.c).unwrap().push(zs.n as usize);
        zs.n += 1;

        let d_req = digest::d(m.clone());
        let h_n = match zs.h.last() {
            None => d_req,
            Some(h_n_minus_1) => digest::d((h_n_minus_1, d_req)),
        };
        zs.h.push(h_n);

        let od = OrderedRequestMessage {
            v: zs.v as u64,
            n: zs.n as u64,
            h: h_n,
            d_req: d_req,
            i: z.me,
            s: m.s,
            nd: Vec::new(),
        };

        n.send_to_all(Message::Signed(Signed::new(
            UnsignedMessage::OrderedRequest(od),
            &z.private_me,
        )));
    }
    return Message::Unsigned(UnsignedMessage::Test(TestMessage { c: z.me }));
}

impl Zeno {
    fn verifier(m: Signed<UnsignedMessage>) -> Option<UnsignedMessage> {
        match m.clone().base {
            UnsignedMessage::Request(rm) => m.verify(&rm.c),
            _ => None,
        }
    }

    fn match_unsigned_message(self, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => Some(on_request_message(self, rm, n)),
            _ => None,
        }
    }

    fn handle_message(self, m: Message, n: Network) -> Option<Message> {
        match m {
            Message::Unsigned(um) => self.match_unsigned_message(um, n),
            Message::Signed(sm) => match Zeno::verifier(sm) {
                None => {
                    println!("Unable to verify message!");
                    None
                }
                Some(u) => self.match_unsigned_message(u, n),
            },
        }
    }
}

pub fn start_zeno(
    url: String,
    kp: signed::KeyPair,
    pubkeys_to_url: HashMap<signed::Public, String>,
) -> Zeno {
    let zeno = Zeno {
        me: kp.clone().0,
        private_me: kp.clone().1,
        state: Arc::new(Mutex::new(ZenoState {
            pubkeys: pubkeys_to_url
                .keys()
                .filter(|&&p| p != kp.0)
                .map(|p| p.clone())
                .collect(),
            n: -1,
            v: 0,
            h: Vec::new(),
            requests: HashMap::new(),
            replies: HashMap::new(),
            status: ZenoStatus::Replica,
            all_requests: Vec::new(),
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
        let mut keypairs: Vec<signed::KeyPair> = Vec::new();
        let mut zenos = Vec::new();
        for i in 0..4 {
            let kp = signed::gen_keys();
            keypairs.push(kp.clone());
            pubkeys_to_urls.insert(kp.0, urls[i].clone());
        }

        for i in 0..4 {
            zenos.push(start_zeno(
                urls[i].clone(),
                keypairs[i].clone(),
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
