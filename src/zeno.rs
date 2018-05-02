use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use digest;
use digest::{HashChain, HashDigest};
use message;
use message::{ClientResponseMessage, ConcreteClientResponseMessage, Message,
              OrderedRequestMessage, RequestMessage, UnsignedMessage};
use signed;
use signed::Signed;
use tcp::Network;

macro_rules! z_debug {
    ($z:expr, $fmt: expr) => {
        println!("{:?} {}", $z.url, $fmt)
    };
    ($z:ident, $fmt: expr, $($arg:expr),*) => {
        println!("{:?} {}", $z.url, format!($fmt, $($arg),*))
    };
}

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

    pending_reqs: HashMap<HashDigest, Sender<OrderedRequestMessage>>,
    pending_ors: Vec<OrderedRequestMessage>,

    status: ZenoStatus,
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
}

#[derive(Clone)]
pub struct Zeno {
    url: String,
    me: signed::Public,
    private_me: signed::Private,
    state: Arc<Mutex<ZenoState>>,
}

fn on_request_message(z: &Zeno, m: &RequestMessage, n: &Network) -> Option<Message> {
    let d_req = digest::d(m);
    let mut zs = z.state.lock().unwrap();
    if !zs.pending_ors.is_empty() && zs.pending_ors[0].d_req == d_req {
        let or = zs.pending_ors.remove(0);
        match check_and_execute_request(z, &mut zs, &or, m, n) {
            Some(rx) => {
                drop(zs);
                let app_resp = rx.recv().unwrap();
                Some(generate_client_response(z, app_resp, &or, m))
            }
            None => None,
        }
    } else {
        match zs.status {
            ZenoStatus::Primary => {
                let or_opt = order_message(z, &mut zs, m, n);
                match or_opt {
                    Some(or) => match check_and_execute_request(z, &mut zs, &or, m, n) {
                        Some(rx) => {
                            drop(zs);
                            let app_resp = rx.recv().unwrap();
                            Some(generate_client_response(z, app_resp, &or, m))
                        }
                        None => None,
                    },
                    None => None,
                }
            }
            ZenoStatus::Replica => {
                let (tx, rx) = mpsc::channel();
                zs.pending_reqs.insert(d_req, tx);
                drop(zs);
                let or = rx.recv().unwrap();
                let mut zs = z.state.lock().unwrap();
                match check_and_execute_request(z, &mut zs, &or, m, n) {
                    Some(rx) => {
                        drop(zs);
                        let app_resp = rx.recv().unwrap();
                        Some(generate_client_response(z, app_resp, &or, m))
                    }
                    None => None,
                }
            }
        }
    }
}

fn order_message(
    z: &Zeno,
    zs: &mut ZenoState,
    m: &RequestMessage,
    n: &Network,
) -> Option<OrderedRequestMessage> {
    let last_t: i64;

    zs.requests.entry(m.c).or_insert_with(Vec::new);
    assert!(zs.all_requests.len() == (zs.n + 1) as usize);

    if zs.requests[&m.c].len() > 0 {
        let last_req_option = zs.requests[&m.c].last();
        let last_req = last_req_option.unwrap();
        last_t = zs.all_requests[*last_req].t as i64;
    } else {
        last_t = -1;
    }

    if last_t + 1 != m.t as i64 {
        None
    } else {
        zs.all_requests.push(m.clone());
        zs.requests.get_mut(&m.c).unwrap().push(zs.n as usize);

        let d_req = digest::d(m.clone());
        let h_n = match zs.h.last() {
            None => d_req,
            Some(h_n_minus_1) => digest::d((h_n_minus_1, d_req)),
        };

        let od = OrderedRequestMessage {
            v: zs.v as u64,
            n: (zs.n + 1) as u64,
            h: h_n,
            d_req: d_req,
            i: z.me.clone(),
            s: m.s,
            nd: Vec::new(),
        };

        let n1 = n.clone();
        let od1 = od.clone();
        let private_me1 = z.private_me.clone();
        thread::spawn(move || {
            println!("Sending OR!");
            let res_map = n1.send_to_all(Message::Signed(Signed::new(
                UnsignedMessage::OrderedRequest(od1),
                &private_me1,
            )));
            println!("OR SEND RESULTS: {:?}", res_map);
            for (_key, val) in res_map {
                val.ok();
            }
            println!("Sent OR!");
        });
        Some(od)
    }
}

fn generate_client_response(
    z: &Zeno,
    app_response: Vec<u8>,
    om: &OrderedRequestMessage,
    msg: &RequestMessage,
) -> Message {
    let crm = ClientResponseMessage {
        response: ConcreteClientResponseMessage::SpecReply(Signed::new(
            message::SpecReplyMessage {
                v: om.v,
                n: om.n,
                h: om.h,
                d_r: digest::d(app_response.clone()),
                c: msg.c,
                t: msg.t,
            },
            &z.private_me,
        )),
        j: z.me,
        r: app_response,
        or: om.clone(),
    };
    Message::Signed(Signed::new(
        UnsignedMessage::ClientResponse(crm),
        &z.private_me,
    ))
}

/// Executes the given request.
/// Performs no checks.
///
/// Returns a message to be sent to the client.
fn check_and_execute_request(
    z: &Zeno,
    zs: &mut ZenoState,
    om: &OrderedRequestMessage,
    msg: &RequestMessage,
    _net: &Network,
) -> Option<Receiver<Vec<u8>>> {
    z_debug!(z, "Executing request!");
    // check valid view
    if om.v != zs.v as u64 {
        z_debug!(z, "Invalid view number: {} != {}", om.v, zs.v);
        return None;
    }
    // check valid sequence number
    if om.n as i64 > zs.n + 1 {
        unimplemented!("got sequence numbers out-of-order");
    }
    // check history digest
    let m_digest = digest::d(msg.clone());
    let history_digest = match zs.h.last() {
        None => m_digest,
        Some(h_n_minus_1) => digest::d((h_n_minus_1, m_digest)),
    };
    if history_digest != om.h {
        z_debug!(
            z,
            "History digests don't match: {:?} {:?}",
            history_digest,
            om.h
        );
        unimplemented!("History digests don't match");
    }

    let (tx, rx) = mpsc::channel();
    zs.apply_tx
        .send((ApplyMsg::Apply(msg.o.clone()), tx))
        .unwrap();
    zs.n += 1;
    zs.h.push(history_digest);
    Some(rx)
}

fn on_ordered_request(z: &Zeno, om: OrderedRequestMessage, _net: Network) {
    let zs = &mut *z.state.lock().unwrap();
    match zs.pending_reqs.remove(&om.d_req) {
        Some(tx) => {
            tx.send(om).unwrap();
        }
        None => {
            zs.pending_ors.push(om);
        }
    }
}

impl Zeno {
    fn verifier(m: Signed<UnsignedMessage>) -> Option<UnsignedMessage> {
        match m.clone().base {
            UnsignedMessage::Request(rm) => m.verify(&rm.c),
            UnsignedMessage::OrderedRequest(or) => m.verify(&or.i),
            _ => None,
        }
    }

    fn match_unsigned_message(&self, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => on_request_message(self, &rm, &n),
            UnsignedMessage::OrderedRequest(orm) => {
                on_ordered_request(self, orm, n);
                None
            }
            _ => None,
        }
    }

    fn handle_message(self, m: Message, n: Network) -> Option<Message> {
        z_debug!(self, "handling message: {:?}", m);
        match m {
            Message::Unsigned(um) => self.match_unsigned_message(um, n),
            Message::Signed(sm) => match Zeno::verifier(sm) {
                None => {
                    z_debug!(self, "Unable to verify message!");
                    None
                }
                Some(u) => {
                    let ret = self.match_unsigned_message(u, n);
                    z_debug!(self, "Returning: {:?}", ret);
                    ret
                }
            },
        }
    }
}

pub fn start_zeno(
    url: String,
    kp: signed::KeyPair,
    pubkeys_to_url: HashMap<signed::Public, String>,
    primary: bool,
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
) -> Zeno {
    let zeno = Zeno {
        url: url.clone(),
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
            status: if primary {
                ZenoStatus::Primary
            } else {
                ZenoStatus::Replica
            },
            all_requests: Vec::new(),
            pending_reqs: HashMap::new(),
            pending_ors: Vec::new(),
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
    use super::ApplyMsg;
    use super::start_zeno;
    use signed;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::thread;
    use std::time;
    use zeno_client;

    #[test]
    fn client_gets_response() {
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
        for i in 0..4 {
            let (tx, rx) = mpsc::channel();
            let mut zeno_pkeys_to_urls = pubkeys_to_urls.clone();
            zeno_pkeys_to_urls.remove(&keypairs[i].0);
            assert!(zeno_pkeys_to_urls.len() == 3);
            zenos.push(start_zeno(
                urls[i].clone(),
                keypairs[i].clone(),
                zeno_pkeys_to_urls,
                i == 0,
                tx,
            ));
            thread::spawn(move || {
                loop {
                    // simple echo application
                    match rx.recv() {
                        Ok((app_msg, tx)) => match app_msg {
                            ApplyMsg::Apply(x) => {
                                tx.send(x).ok();
                            }
                        },
                        Err(_) => break,
                    }
                }
            });
        }
        let (tx, rx) = mpsc::channel();
        let t = thread::spawn(move || {
            // give the servers some time to know each other
            thread::sleep(time::Duration::new(1, 100));
            let mut c = zeno_client::Client::new(signed::gen_keys(), pubkeys_to_urls.clone());
            c.request(vec![], false);
            tx.send(()).unwrap();
        });
        assert_eq!(rx.recv_timeout(time::Duration::from_secs(5)), Ok(()));
        t.join().unwrap();
    }
}
