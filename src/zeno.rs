use chrono::Utc;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use digest;
use digest::{HashChain, HashDigest};
use message;
use message::{ClientResponseMessage, CommitCertificate, CommitMessage,
              ConcreteClientResponseMessage, IHateThePrimaryMessage, Message, NewViewMessage,
              OrderedRequestMessage, RequestMessage, UnsignedMessage, ViewChangeMessage};
use signed;
use signed::Signed;
use tcp::Network;

const IHATETHEPRIMARY_TIMEOUT: Duration = Duration::from_secs(1);

macro_rules! z_debug {
    ($z:expr, $fmt: expr) => {
        let time_str = Utc::now().format("%T%.3f");
        println!("[{}] {} {}", time_str, $z.url, $fmt)
    };
    ($z:ident, $fmt: expr, $($arg:expr),*) => {
        let time_str = Utc::now().format("%T%.3f");
        println!("[{}] {} {}", time_str, $z.url, format!($fmt, $($arg),*))
    };
}

pub enum ApplyMsg {
    Apply(Vec<u8>),
}

#[derive(Clone, Copy)]
enum ViewState {
    ViewActive,
    ViewChanging,
}

/// stores the mutable state of our zeno server
#[allow(dead_code)]
struct ZenoState {
    pubkeys: Vec<signed::Public>,

    // See 4.3
    n: i64,
    v: u64,
    h: HashChain,
    // highest timestamp we've received from given client
    highest_t_received: HashMap<signed::Public, u64>,
    // all requests we've executed for each client
    // (index in all_executed_reqs)
    executed_reqs: HashMap<signed::Public, Vec<usize>>,
    // the latest client response we've sent to each client
    replies: HashMap<signed::Public, Option<Message>>,

    // all client requests we've executed, in order
    all_executed_reqs: Vec<(OrderedRequestMessage, RequestMessage)>,

    // channels for threads handling requests without ORs
    reqs_without_ors: HashMap<HashDigest, Sender<OrderedRequestMessage>>,
    // channels for threads handling requests without COMMITs
    reqs_without_commits: HashMap<HashDigest, Sender<CommitMessage>>,
    // ORs received for requests we haven't gotten yet. sorted
    pending_ors: Vec<OrderedRequestMessage>,
    // COMMITs received for requests we haven't gotten yet
    pending_commits: HashMap<HashDigest, Vec<CommitMessage>>,

    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,

    last_cc: CommitCertificate,

    ihatetheprimary_timer_stopper: Option<Sender<()>>,
    ihatetheprimary_accusations: HashSet<signed::Public>,
    view_changes: HashMap<signed::Public, ViewChangeMessage>,
    view_state: ViewState,

    new_views: HashMap<signed::Public, NewViewMessage>,
}

fn get_primary_for_view(zs: &ZenoState, v: usize) -> signed::Public {
    zs.pubkeys[v as usize % zs.pubkeys.len()]
}

fn get_primary(zs: &ZenoState) -> signed::Public {
    get_primary_for_view(zs, zs.v as usize)
}

fn is_primary(z: &Zeno, zs: &ZenoState) -> bool {
    get_primary(zs) == z.me
}

/// represents the entire state of our zeno server
#[derive(Clone)]
pub struct Zeno {
    url: String,
    me: signed::Public,
    private_me: signed::Private,
    max_failures: u64,
    state: Arc<Mutex<ZenoState>>,
}

fn already_received_msg(zs: &ZenoState, msg: &RequestMessage) -> bool {
    match zs.highest_t_received.get(&msg.c) {
        Some(&t) => msg.t <= t,
        None => false,
    }
}

/// returns whether we've already handled the given client request already
/// (specifically, whether the last request in zs.all_executed_reqs is newer than
/// msg)
fn already_handled_msg(zs: &ZenoState, msg: &RequestMessage) -> bool {
    match zs.executed_reqs.get(&msg.c) {
        Some(reqs) => match reqs.last() {
            Some(&last_req_n) => zs.all_executed_reqs[last_req_n].1.t >= msg.t,
            None => false,
        },
        None => false,
    }
}

/// returns a signed message representing msg signed with priv_key
fn get_signed_message(msg: UnsignedMessage, priv_key: &signed::Private) -> Message {
    Message::Signed(Signed::new(msg, priv_key))
}

fn broadcast(net: Network, msg: Message) {
    let res_map = net.send_to_all(msg);
    for (_key, val) in res_map {
        val.ok();
    }
}

fn start_ihatetheprimary_timer(z: &Zeno, zs: &mut ZenoState, net: &Network) {
    let (tx, rx) = mpsc::channel();
    zs.ihatetheprimary_timer_stopper = Some(tx);
    let z1 = z.clone();
    let net1 = net.clone();
    thread::spawn(move || match rx.recv_timeout(IHATETHEPRIMARY_TIMEOUT) {
        Err(_) => {
            let mut zs1 = z1.state.lock().unwrap();
            zs1.ihatetheprimary_accusations.insert(z1.me);

            let msg = get_signed_message(
                UnsignedMessage::IHateThePrimary(IHateThePrimaryMessage { v: zs1.v, i: z1.me }),
                &z1.private_me,
            );
            let z2 = z1.clone();
            thread::spawn(move || {
                broadcast(net1, msg);
                z_debug!(z2, "Broadcasted IHATETHEPRIMARY!");
            });
        }
        _ => {}
    });
}

/// given a client request, does lots of stuff
fn on_request_message(z: &Zeno, m: &RequestMessage, net: &Network) -> Option<Message> {
    z_debug!(z, "Got request message");
    let d_req = digest::d(m);
    let mut zs = z.state.lock().unwrap();
    if already_received_msg(&zs, m) {
        z_debug!(z, "Already received msg {:?}", m);
        if already_handled_msg(&zs, m) {
            return zs.replies.get(&m.c).unwrap().clone();
        } else {
            z_debug!(z, "starting IHTP timer");
            start_ihatetheprimary_timer(z, &mut zs, net);
        }
    }
    if !zs.pending_ors.is_empty() && zs.pending_ors[0].d_req == d_req
        && zs.pending_ors[0].n == (zs.n + 1) as u64
    {
        let or = zs.pending_ors.remove(0);
        check_and_execute_request(z, zs, &or, m, net)
    } else {
        if is_primary(z, &zs) {
            let or_opt = order_message(z, &mut zs, m, net);
            match or_opt {
                Some(or) => check_and_execute_request(z, zs, &or, m, net),
                None => None,
            }
        } else {
            let (tx, rx) = mpsc::channel();
            zs.reqs_without_ors.insert(d_req.clone(), tx);
            drop(zs);
            let or = rx.recv().unwrap();
            let mut zs = z.state.lock().unwrap();
            zs.reqs_without_ors.remove(&d_req);
            check_and_execute_request(z, zs, &or, m, net)
        }
    }
}

/// Takes the given message and returns an OrderedRequestMessage to be applied.
/// Also starts a thread to send the OrderedRequestMessage to all servers.
/// Does not apply the message.
fn order_message(
    z: &Zeno,
    zs: &mut ZenoState,
    m: &RequestMessage,
    n: &Network,
) -> Option<OrderedRequestMessage> {
    let last_t: i64;

    zs.executed_reqs.entry(m.c).or_insert_with(Vec::new);
    assert!(zs.all_executed_reqs.len() == (zs.n + 1) as usize);

    if zs.executed_reqs[&m.c].len() > 0 {
        let last_req_option = zs.executed_reqs[&m.c].last();
        let last_req = last_req_option.unwrap();
        last_t = zs.all_executed_reqs[*last_req].1.t as i64;
    } else {
        last_t = -1;
    }

    if last_t + 1 != m.t as i64 {
        None
    } else {
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
            let res_map = n1.send_to_all(get_signed_message(
                UnsignedMessage::OrderedRequest(od1),
                &private_me1,
            ));
            for (_key, val) in res_map {
                val.ok();
            }
            println!("Sent OR!");
        });
        Some(od)
    }
}

/// Generates a client response based on a request and orderedrequest
///
/// Assumes that we've already verified om and msg and they are correct.
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

/// Runs checks and executes the given request.
///
/// If the request checks out, this method sends it to the
/// application and returns a channel receiver on which the application
/// will send its response.
fn check_and_execute_request(
    z: &Zeno,
    mut zs: MutexGuard<ZenoState>,
    or: &OrderedRequestMessage,
    msg: &RequestMessage,
    net: &Network,
) -> Option<Message> {
    z_debug!(z, "Executing request!");
    // check valid view
    if or.v != zs.v as u64 {
        z_debug!(z, "Invalid view number: {} != {}", or.v, zs.v);
        return None;
    }
    // check valid sequence number
    if or.n as i64 > zs.n + 1 {
        unimplemented!("got sequence numbers out-of-order");
    }
    // check history digest
    let m_digest = digest::d(msg.clone());
    let history_digest = match zs.h.last() {
        None => m_digest,
        Some(h_n_minus_1) => digest::d((h_n_minus_1, m_digest)),
    };
    if history_digest != or.h {
        z_debug!(
            z,
            "History digests don't match: {:?} {:?}",
            history_digest,
            or.h
        );
        unimplemented!("History digests don't match");
    }

    // checks done, let's execute
    let (tx, rx) = mpsc::channel();
    zs.apply_tx
        .send((ApplyMsg::Apply(msg.o.clone()), tx))
        .unwrap();

    zs.n += 1;
    z_debug!(z, "Executed request {}", zs.n);
    zs.all_executed_reqs.push((or.clone(), msg.clone()));
    zs.executed_reqs.entry(msg.c).or_insert_with(Vec::new);
    let n = zs.n as usize;
    zs.executed_reqs.get_mut(&msg.c).unwrap().push(n);
    assert!(zs.all_executed_reqs.len() - 1 == zs.n as usize);
    zs.h.push(history_digest);

    // if we already have a next request queued, let that thread know
    if zs.pending_ors.len() > 0 && zs.pending_ors[0].n == (zs.n + 1) as u64 {
        let d_req = zs.pending_ors[0].d_req.clone();
        if let Some(tx_next) = zs.reqs_without_ors.remove(&d_req) {
            z_debug!(z, "Sending next request OR for {}", zs.n + 1);
            tx_next.send(zs.pending_ors.remove(0)).unwrap();
        }
    }

    if msg.s {
        let mut commit_cert = zs.pending_commits
            .get(&or.d_req)
            .unwrap_or(&Vec::new())
            .into_iter()
            .filter(|commit| commit.or == *or)
            .map(|commit| commit.clone())
            .collect::<HashSet<CommitMessage>>();
        zs.pending_commits.remove(&or.d_req);

        let (tx_commit, rx_commit) = mpsc::channel();
        zs.reqs_without_commits.insert(or.d_req.clone(), tx_commit);
        drop(zs);

        let n1 = net.clone();
        let commit_msg = get_signed_message(
            UnsignedMessage::Commit(CommitMessage {
                or: or.clone(),
                j: z.me,
            }),
            &z.private_me,
        );
        thread::spawn(move || {
            let res_map = n1.send_to_all(commit_msg);
            for (_key, val) in res_map {
                val.ok();
            }
            println!("Sent COMMIT!");
        });

        while commit_cert.len() < 2 * z.max_failures as usize {
            let commit = rx_commit.recv().unwrap();
            if commit.or == *or {
                commit_cert.insert(commit);
            }
        }

        let mut zs1 = z.state.lock().unwrap();
        zs1.reqs_without_commits.remove(&or.d_req);
        zs1.last_cc = commit_cert.into_iter().collect();
        zs1.last_cc.sort_by(|ref a, ref b| a.j.cmp(&(b.j)));
    } else {
        drop(zs);
    }

    let app_resp = rx.recv().unwrap();
    Some(generate_client_response(z, app_resp, &or, msg))
}

fn on_ordered_request(z: &Zeno, or: OrderedRequestMessage, _net: Network) {
    let zs = &mut *z.state.lock().unwrap();
    if or.v != zs.v {
        z_debug!(z, "Ignoring OR from v:{} because v is {}", or.v, zs.v);
        return;
    }
    if or.i != get_primary(zs) {
        z_debug!(z, "Ignoring OR from wrong primary for this view");
        return;
    }
    process_or(zs, or);
}

fn queue_or(zs: &mut ZenoState, or: OrderedRequestMessage) {
    match zs.pending_ors.binary_search_by_key(&or.n, |o| o.n) {
        Ok(_) => {},
        Err(i) => {
            zs.pending_ors.insert(i, or);
        }
    }
}

fn process_or(zs: &mut ZenoState, or: OrderedRequestMessage) {
    if or.n as i64 <= zs.n {
        return;
    }
    match zs.reqs_without_ors.remove(&or.d_req) {
        Some(tx) => {
            if or.n == (zs.n + 1) as u64 {
                tx.send(or).unwrap();
            } else {
                zs.reqs_without_ors.insert(or.d_req.clone(), tx);
                queue_or(zs, or);
            }
        }
        None => {
            if !zs.pending_ors.iter().any(|pending_or| *pending_or == or) {
                queue_or(zs, or);
            }
        }
    }
}

fn on_commit(z: &Zeno, cm: CommitMessage) {
    let zs = &mut *z.state.lock().unwrap();
    match zs.reqs_without_commits.get(&cm.or.d_req) {
        Some(tx) => {
            z_debug!(z, "Sending commit to existing thread");
            tx.send(cm).ok();
        }
        None => {
            z_debug!(z, "queueing commit");
            zs.pending_commits
                .entry(cm.or.d_req.clone())
                .or_insert_with(Vec::new);
            zs.pending_commits.get_mut(&cm.or.d_req).unwrap().push(cm);
        }
    }
}

fn is_view_active(zs: &ZenoState) -> bool {
    match zs.view_state {
        ViewState::ViewActive => true,
        ViewState::ViewChanging => false,
    }
}

fn on_ihatetheprimary(z: &Zeno, msg: IHateThePrimaryMessage, net: Network) {
    let mut zs = z.state.lock().unwrap();
    if msg.v == zs.v {
        zs.ihatetheprimary_accusations.insert(msg.i);
        // switch the view from active to inactive if we've gotten enough
        if is_view_active(&zs) {
            let num_accusations = zs.ihatetheprimary_accusations.len();
            if num_accusations >= z.max_failures as usize + 1 {
                zs.view_state = ViewState::ViewChanging;
                zs.v += 1;
                let o = match zs.last_cc.get(0) {
                    Some(cm) => {
                        let last_committed = cm.or.n as usize;
                        zs.all_executed_reqs[last_committed + 1..]
                            .iter()
                            .map(|x| x.0.clone())
                            .collect()
                    }
                    None => Vec::new(),
                };
                let msg = Message::Signed(Signed::new(
                    UnsignedMessage::ViewChange(message::ViewChangeMessage {
                        v: zs.v,
                        cc: zs.last_cc.clone(),
                        o: o,
                        i: z.me,
                    }),
                    &z.private_me,
                ));
                thread::spawn(move || {
                    broadcast(net, msg);
                });
            }
        }
    }
}

fn apply_new_view(_z: &Zeno, zs: &mut ZenoState, nvm: &NewViewMessage) {
    for view_change in nvm.p.iter() {
        for orm in view_change.o.iter() {
            process_or(zs, orm.clone());
        }
        let new_view_cc_n = match view_change.cc.first() {
            Some(cm) => cm.or.n as i64,
            None => -1,
        };
        let our_cc_n = match zs.last_cc.first() {
            Some(cm) => cm.or.n as i64,
            None => -1,
        };
        if new_view_cc_n > our_cc_n {
            zs.last_cc = view_change.cc.clone();
        }
    }
    zs.v = nvm.v;
}

fn on_viewchange(z: &Zeno, msg: ViewChangeMessage, net: Network) {
    let zs = &mut *z.state.lock().unwrap();
    if msg.v == zs.v {
        zs.view_changes.insert(msg.i, msg);
        match zs.view_state {
            ViewState::ViewActive => {
                if zs.view_changes.len() >= z.max_failures as usize * 2 + 1 {
                    if is_primary(z, zs) {
                        let nvm = NewViewMessage {
                            v: zs.v,
                            p: zs.view_changes.values().cloned().collect(),
                            i: z.me,
                        };
                        apply_new_view(z, zs, &nvm);
                        let msg = Message::Signed(Signed::new(
                            UnsignedMessage::NewView(nvm),
                            &z.private_me,
                        ));
                        broadcast(net, msg);
                    }
                }
            }
            ViewState::ViewChanging => {}
        }
    }
}

fn on_newview(z: &Zeno, msg: NewViewMessage, _net: Network) {
    let zs = &mut *z.state.lock().unwrap();
    if get_primary_for_view(zs, msg.v as usize) != msg.i {
        // the sender was not the right primary for this view
        return;
    }

    //TODO: checks
    apply_new_view(z, zs, &msg);
    // TODO: to support weak requests, merge all messages in p
    // with the merge protocol
}

impl Zeno {
    pub fn verifier(m: Signed<UnsignedMessage>) -> Option<UnsignedMessage> {
        match m.clone().base {
            UnsignedMessage::Request(rm) => m.verify(&rm.c),
            UnsignedMessage::OrderedRequest(or) => m.verify(&or.i),
            UnsignedMessage::ClientResponse(crm) => m.verify(&crm.j),
            UnsignedMessage::Commit(cm) => m.verify(&cm.j),
            UnsignedMessage::IHateThePrimary(ihtpm) => m.verify(&ihtpm.i),
            UnsignedMessage::ViewChange(vcm) => m.verify(&vcm.i),
            UnsignedMessage::NewView(nvm) => m.verify(&nvm.i),
            _ => None,
        }
    }

    fn match_unsigned_message(&self, m: UnsignedMessage, n: Network) -> Option<Message> {
        match m {
            UnsignedMessage::Request(rm) => {
                let reply_opt = on_request_message(self, &rm, &n);
                match reply_opt.clone() {
                    Some(reply) => {
                        let mut zs = self.state.lock().unwrap();
                        zs.replies.insert(rm.c, Some(reply));
                    }
                    None => {}
                };
                reply_opt
            }
            UnsignedMessage::OrderedRequest(orm) => {
                on_ordered_request(self, orm, n);
                None
            }
            UnsignedMessage::Commit(cm) => {
                z_debug!(self, "GOT COMMIT!");
                on_commit(self, cm);
                None
            }
            UnsignedMessage::IHateThePrimary(ihtpm) => {
                z_debug!(self, "GOT IHTP");
                on_ihatetheprimary(self, ihtpm, n);
                None
            }

            UnsignedMessage::ViewChange(vcm) => {
                z_debug!(self, "GOT ViewChange");
                on_viewchange(self, vcm, n);
                None
            }
            UnsignedMessage::NewView(nvm) => {
                z_debug!(self, "GOT NewView");
                on_newview(self, nvm, n);
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
                    z_debug!(self, "Unable to verify message!");
                    None
                }
                Some(u) => {
                    let ret = self.match_unsigned_message(u, n);
                    ret
                }
            },
        }
    }
}

pub fn start_zeno(
    url: String,
    kp: signed::KeyPair,
    pubkeys: Vec<signed::Public>,
    pubkeys_to_url: HashMap<signed::Public, String>,
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
    max_failures: u64,
) -> Zeno {
    let mut pubkeys_to_url_without_me = pubkeys_to_url.clone();
    pubkeys_to_url_without_me.remove(&kp.0);
    let zeno = Zeno {
        url: url.clone(),
        me: kp.clone().0,
        private_me: kp.clone().1,
        max_failures: max_failures,
        state: Arc::new(Mutex::new(ZenoState {
            pubkeys: pubkeys,
            n: -1,
            v: 0,
            h: Vec::new(),
            highest_t_received: HashMap::new(),
            executed_reqs: HashMap::new(),
            replies: HashMap::new(),
            all_executed_reqs: Vec::new(),
            reqs_without_ors: HashMap::new(),
            reqs_without_commits: HashMap::new(),
            pending_ors: Vec::new(),
            pending_commits: HashMap::new(),
            apply_tx: apply_tx,
            last_cc: Vec::new(),
            ihatetheprimary_timer_stopper: None,
            ihatetheprimary_accusations: HashSet::new(),
            view_changes: HashMap::new(),
            view_state: ViewState::ViewActive,
            new_views: HashMap::new(),
        })),
    };
    Network::new(
        url,
        pubkeys_to_url_without_me,
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
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::sync::mpsc::Receiver;
    use std::thread;
    use std::time;
    use zeno_client;

    trait TestApp {
        fn new() -> Self;

        fn apply(&mut self, input: ApplyMsg) -> Vec<u8>;
    }

    struct EchoApp;
    impl TestApp for EchoApp {
        fn new() -> EchoApp {
            EchoApp {}
        }
        fn apply(&mut self, input: ApplyMsg) -> Vec<u8> {
            match input {
                ApplyMsg::Apply(x) => x,
            }
        }
    }

    struct CountApp {
        x: u8,
    }
    impl TestApp for CountApp {
        fn new() -> CountApp {
            CountApp { x: 0 }
        }
        fn apply(&mut self, input: ApplyMsg) -> Vec<u8> {
            match input {
                ApplyMsg::Apply(y) => {
                    assert_eq!(y.len(), 1);
                    self.x += y[0];
                    vec![self.x]
                }
            }
        }
    }

    #[test]
    fn test_one_message() {
        test_input_output(vec![vec![1, 2, 3]], vec![vec![1, 2, 3]], 4, 1, false, 1);
    }

    #[test]
    fn test_one_message_strong() {
        test_input_output(vec![vec![1, 2, 3]], vec![vec![1, 2, 3]], 4, 1, true, 1);
    }

    #[test]
    fn test_multiple_messages() {
        test_input_output(
            vec![vec![1], vec![2]],
            vec![vec![1], vec![2]],
            4,
            1,
            false,
            1,
        );
    }

    #[test]
    fn test_multiple_messages_strong() {
        test_input_output(
            vec![vec![1], vec![2]],
            vec![vec![1], vec![2]],
            4,
            1,
            true,
            1,
        );
    }

    #[test]
    fn test_many_messages_strong() {
        let input: Vec<_> = (0..100).map(|i| vec![i]).collect();
        let output = input.clone();
        test_input_output(input, output, 4, 1, true, 1);
    }

    fn port_adj() -> u16 {
        loop {
            let a = TcpListener::bind("127.0.0.1:0");
            if a.is_ok() {
                return a.unwrap().local_addr().unwrap().port();
            } else {
                drop(a);
            }
        }
    }

    fn start_zenos<T: TestApp>(
        num_servers: usize,
        max_failures: usize,
        unresponsive_failures: Vec<usize>,
    ) -> (HashMap<signed::Public, String>) {
        let mut urls = Vec::new();
        for _ in 0..num_servers {
            urls.push(format!("127.0.0.1:{}", port_adj()));
        }
        let mut pubkeys_to_urls = HashMap::new();
        let mut keypairs: Vec<signed::KeyPair> = Vec::new();

        for i in 0..num_servers {
            let kp = signed::gen_keys();
            keypairs.push(kp.clone());
            pubkeys_to_urls.insert(kp.0, urls[i].clone());
        }

        let mut zenos = Vec::new();
        for i in 0..num_servers {
            if unresponsive_failures.contains(&i) {
                // just don't start this zeno
                continue;
            }
            let (tx, rx) = mpsc::channel();
            zenos.push(start_zeno(
                urls[i].clone(),
                keypairs[i].clone(),
                keypairs.iter().map(|kp| kp.0).collect(),
                pubkeys_to_urls.clone(),
                tx,
                max_failures as u64,
            ));
            thread::spawn(move || {
                let mut app = T::new();
                loop {
                    // simple echo application
                    match rx.recv() {
                        Ok((app_msg, tx)) => {
                            tx.send(app.apply(app_msg)).ok();
                        }
                        Err(_) => break,
                    }
                }
            });
        }
        pubkeys_to_urls
    }

    #[test]
    fn test_one_client_strong_consistency() {
        let pubkeys_to_urls = start_zenos::<CountApp>(4, 1, vec![]);

        let rx = do_ops_as_new_client(
            pubkeys_to_urls,
            vec![vec![1]; 100],
            Some((1..101).map(|x| vec![x]).collect()),
            1,
            true,
        );
        assert_eq!(rx.recv_timeout(time::Duration::new(30, 0)), Ok(()));
    }

    #[test]
    fn test_one_client_strong_consistency_replica_fail() {
        let pubkeys_to_urls = start_zenos::<CountApp>(4, 1, vec![2]);

        let rx = do_ops_as_new_client(
            pubkeys_to_urls,
            vec![vec![1]; 100],
            Some((1..101).map(|x| vec![x]).collect()),
            1,
            true,
        );
        assert_eq!(rx.recv_timeout(time::Duration::new(30, 0)), Ok(()));
    }

    #[test]
    fn test_one_client_strong_consistency_primary_fail() {
        let pubkeys_to_urls = start_zenos::<CountApp>(4, 1, vec![0]);

        let rx = do_ops_as_new_client(
            pubkeys_to_urls,
            vec![vec![1]; 100],
            Some((1..101).map(|x| vec![x]).collect()),
            1,
            true,
        );
        assert_eq!(rx.recv_timeout(time::Duration::new(30, 0)), Ok(()));
    }

    fn do_ops_as_new_client(
        pubkeys_to_urls: HashMap<signed::Public, String>,
        ops: Vec<Vec<u8>>,
        expected: Option<Vec<Vec<u8>>>,
        max_failures: u64,
        strong: bool,
    ) -> Receiver<()> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // give the servers some time to know each other
            // TODO: detect stabilization rather than sleep
            thread::sleep(time::Duration::new(2, 0));
            let mut c = zeno_client::Client::new(signed::gen_keys(), pubkeys_to_urls, max_failures);
            for (i, op) in ops.into_iter().enumerate() {
                let result = c.request(op, strong);
                if let Some(e) = expected.as_ref() {
                    assert_eq!(e[i], result);
                }
            }
            tx.send(()).unwrap();
        });
        return rx;
    }

    // waits on a bunch of channels, rxs
    // asserts that no channel takes longer than timeout
    fn wait_on_all(rxs: Vec<Receiver<()>>, timeout: time::Duration) {
        let start = time::Instant::now();
        for rx in rxs {
            assert_eq!(rx.recv_timeout(timeout - start.elapsed()), Ok(()));
        }
    }

    #[test]
    fn test_many_clients_strong_consistency() {
        let pubkeys_to_urls = start_zenos::<CountApp>(4, 1, vec![]);

        let mut client_rxs = Vec::new();
        for _ in 0..5 {
            client_rxs.push(do_ops_as_new_client(
                pubkeys_to_urls.clone(),
                vec![vec![1]; 20],
                None,
                1,
                true,
            ));
        }
        wait_on_all(client_rxs, time::Duration::from_secs(30));
        let rx = do_ops_as_new_client(
            pubkeys_to_urls.clone(),
            vec![vec![0]],
            Some(vec![vec![100]]),
            1,
            true,
        );
        rx.recv_timeout(time::Duration::from_secs(30)).unwrap();
    }

    fn test_input_output(
        input: Vec<Vec<u8>>,
        output: Vec<Vec<u8>>,
        num_servers: usize,
        max_failures: usize,
        strong: bool,
        num_clients: usize,
    ) {
        let pubkeys_to_urls = start_zenos::<EchoApp>(num_servers, max_failures, vec![]);
        let mut client_rxs = Vec::new();
        for _ in 0..num_clients {
            client_rxs.push(do_ops_as_new_client(
                pubkeys_to_urls.clone(),
                input.clone(),
                Some(output.clone()),
                max_failures as u64,
                strong,
            ));
        }
        let start = time::Instant::now();
        let total_timeout = time::Duration::from_secs(30);
        for rx in client_rxs {
            assert_eq!(rx.recv_timeout(total_timeout - start.elapsed()), Ok(()));
        }
    }
}
