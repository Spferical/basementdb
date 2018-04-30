use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::mpsc;

use digest;
use digest::{HashChain, HashDigest};
use message;
use message::{ClientResponseMessage, Message, OrderedRequestMessage, RequestMessage, TestMessage,
              UnsignedMessage, ConcreteClientResponseMessage};
use signed;
use signed::Signed;
use tcp::Network;

#[allow(dead_code)]
enum ZenoStatus {
    Replica,
    Primary,
}

pub enum ApplyMsg {
    Apply(Vec<u8>),
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

    reqs_without_or: HashMap<HashDigest, RequestMessage>,
    //TODO: handle case of getting req after or
    ors_without_req: Vec<OrderedRequestMessage>,

    status: ZenoStatus,
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
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

/// As a replica, process the given request.
/// This handles everything replica-side described in Zeno section 4.4.
///
/// Returns a message to be sent to the client.
fn process_request(
    z: Zeno,
    om: OrderedRequestMessage,
    msg: RequestMessage,
    _net: Network,
) -> Option<ClientResponseMessage> {
    let rx: Receiver<Vec<u8>>;
    {
        let mut zs = z.state.lock().unwrap();
        // check valid view
        if om.v != zs.v as u64 {
            return None;
        }
        // check valid sequence number
        if om.n as i64 > zs.n + 1 {
            unimplemented!("got sequence numbers out-of-order");
        }
        // check history digest
        let m_digest = digest::d(msg.clone());
        let history_digest = digest::d((zs.h.clone(), m_digest));
        if history_digest != om.h {
            unimplemented!("History digests don't match");
        }

	let (tx, rx1) = mpsc::channel();
	rx = rx1;
        zs.apply_tx.send((ApplyMsg::Apply(msg.o), tx)).unwrap();
        zs.n += 1;
        zs.h.push(history_digest);
    }
    let app_response = rx.recv().unwrap();
    {
        let zs = z.state.lock().unwrap();
        if !msg.s {
            return Some(
                ClientResponseMessage {
                    response: ConcreteClientResponseMessage::SpecReply(
                        Signed::new(
                            message::SpecReplyMessage {
                                v: zs.v as u64,
                                n: zs.n as u64,
                                h: om.h,
                                d_r: digest::d(app_response.clone()),
                                c: msg.c,
                                t: msg.t,
                            },
                            &z.private_me,
                        )
                    ),
                    j: z.me,
                    r: app_response,
                    or: om,
                }
            );

        } else {
            unimplemented!("Strong request");
        }
    }
}

fn on_ordered_request(z: Zeno, om: OrderedRequestMessage, net: Network) {
    let req_opt = {
        let zs = &mut *z.state.lock().unwrap();
        zs.reqs_without_or.remove(&om.d_req)
    };
    match req_opt {
        Some(req) => {
            println!("{:?}", req);
            process_request(z, om, req, net);
        }
        None => {}
    }
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
            UnsignedMessage::OrderedRequest(orm) => {
                on_ordered_request(self, orm, n);
                None
            }
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
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
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
            reqs_without_or: HashMap::new(),
            ors_without_req: Vec::new(),
            apply_tx: apply_tx,
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
    fn client_gets_reponse() {
        let urls = vec![
            "127.0.0.1:44441".to_string(),
            "127.0.0.1:44442".to_string(),
            "127.0.0.1:44443".to_string(),
            "127.0.0.1:44444".to_string(),
        ];
        let mut pubkeys_to_urls = HashMap::new();
        let mut keypairs: Vec<signed::KeyPair> = Vec::new();
        for i in 0..4 {
            let kp = signed::gen_keys();
            keypairs.push(kp.clone());
            pubkeys_to_urls.insert(kp.0, urls[i].clone());
        }

        let mut zenos = Vec::new();
        let mut apply_rxs = Vec::new();
        for i in 0..4 {
            let (tx, rx) = mpsc::channel();
            apply_rxs.push(rx);
            zenos.push(start_zeno(
                urls[i].clone(),
                keypairs[i].clone(),
                pubkeys_to_urls.clone(),
                tx,
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
