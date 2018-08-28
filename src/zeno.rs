#![cfg_attr(feature = "cargo-clippy", allow(print_literal))]

use chrono::Utc;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use digest;
use digest::{HashChain, HashDigest};
use message;
use message::{
    ClientResponseMessage, CommitCertificate, CommitMessage, ConcreteClientResponseMessage,
    FillHoleMessage, IHateThePrimaryMessage, Message, NewViewMessage, OrderedRequestMessage,
    RequestMessage, ViewChangeMessage,
};
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

enum MessageTarget {
    All,
    One(signed::Public),
}

/// stores the mutable state of our zeno server
#[allow(dead_code)]
struct ZenoMutState {
    alive: bool,
    pubkeys: Vec<signed::Public>,

    // See 4.3
    n: i64,
    v: u64,
    h: HashChain,
    // all requests we've executed for each client
    // (index in all_executed_reqs)
    executed_reqs: HashMap<signed::Public, Vec<usize>>,
    // the latest client response we've sent to each client
    replies: HashMap<signed::Public, Option<Message>>,

    // all client requests we've executed, in order
    all_executed_reqs: Vec<(Signed<OrderedRequestMessage>, Signed<RequestMessage>)>,

    // channels for threads handling requests without ORs
    reqs_without_ors: HashMap<HashDigest, Sender<Signed<OrderedRequestMessage>>>,
    // channels for threads handling requests without COMMITs
    reqs_without_commits: HashMap<HashDigest, Sender<Signed<CommitMessage>>>,
    // ORs received for requests we haven't gotten yet. sorted
    pending_ors: Vec<Signed<OrderedRequestMessage>>,
    // COMMITs received for requests we haven't gotten yet
    pending_commits: HashMap<HashDigest, Vec<Signed<CommitMessage>>>,
    // channel to send messages to application
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
    // highest commit certificate we've received
    last_cc: CommitCertificate,
    // if ihatetheprimary timer is active, sending to this stops it
    ihatetheprimary_timer_stopper: Option<Sender<()>>,
    // accusations we've received for current view
    ihatetheprimary_accusations: HashSet<signed::Public>,
    // view change messages we've received for current view
    view_changes: HashMap<signed::Public, ViewChangeMessage>,
    // whether the current view is active or changing
    view_state: ViewState,
    // new view messages we've received for next view
    new_views: HashMap<signed::Public, NewViewMessage>,
}

fn get_primary_for_view(zs: &ZenoMutState, v: usize) -> signed::Public {
    zs.pubkeys[v as usize % zs.pubkeys.len()]
}

fn get_primary(zs: &ZenoMutState) -> signed::Public {
    get_primary_for_view(zs, zs.v as usize)
}

fn is_primary(z: &ZenoState, zs: &ZenoMutState) -> bool {
    get_primary(zs) == z.me
}

/// represents the entire state of our zeno server
#[derive(Clone)]
pub struct ZenoState {
    url: String,
    me: signed::Public,
    private_me: signed::Private,
    max_failures: u64,
    network_channel: Sender<(MessageTarget, Message)>,
    mut_state: Arc<Mutex<ZenoMutState>>,
}

pub struct Zeno {
    state: ZenoState,
    network: Network,
}

impl Drop for Zeno {
    fn drop(&mut self) {
        self.network.halt();
        self.state.halt();
    }
}

/// returns whether we've already handled the given client request already
/// (specifically, whether the last request in zs.all_executed_reqs is newer than
/// msg)
fn already_handled_msg(zs: &ZenoMutState, msg: &RequestMessage) -> bool {
    match zs.executed_reqs.get(&msg.c) {
        Some(reqs) => match reqs.last() {
            Some(&last_req_n) => zs.all_executed_reqs[last_req_n].1.base.t >= msg.t,
            None => false,
        },
        None => false,
    }
}

fn broadcast(net: &Network, msg: &Message) {
    let res_map = net.send_to_all(msg);
    for (_key, val) in res_map {
        val.ok();
    }
}

/// Sends messages from a channel over the network.
/// Halts when the channel closes.
fn send_from_channel(net: &Network, channel: Receiver<(MessageTarget, Message)>) {
    for (target, msg) in channel {
        match target {
            MessageTarget::One(pubkey) => {
                net.send(&msg, &pubkey).ok();
            }
            MessageTarget::All => {
                broadcast(net, &msg);
            }
        };
    }
}

/// stops any active ihatetheprimary timer
fn stop_ihatetheprimary_timer(zs: &mut ZenoMutState) {
    if let Some(ref tx) = zs.ihatetheprimary_timer_stopper.take() {
        // this may fail if timer ended and thread exited, so no unwrap()
        tx.send(()).ok();
    }
}

fn start_ihatetheprimary_timer(z: &ZenoState, zs: &mut ZenoMutState) {
    z_debug!(z, "starting IHTP timer");
    let (tx, rx) = mpsc::channel();
    zs.ihatetheprimary_timer_stopper = Some(tx);
    let z1 = z.clone();
    let timer_view = zs.v;
    thread::spawn(move || {
        if rx.recv_timeout(IHATETHEPRIMARY_TIMEOUT).is_err() {
            let mut zs1 = z1.mut_state.lock().unwrap();
            if zs1.v != timer_view {
                // view changed while timer running
                return;
            }
            zs1.ihatetheprimary_accusations.insert(z1.me);
            let msg =
                Message::IHateThePrimary(z1.sign(IHateThePrimaryMessage { v: zs1.v, i: z1.me }));
            z1.send_to_all(msg);
            z_debug!(z1, "Broadcasted IHATETHEPRIMARY!");
        }
    });
}

/// given a client request, does lots of stuff
fn on_request_message(z: &ZenoState, sm: &Signed<RequestMessage>) -> Option<Message> {
    let m = &sm.base;
    z_debug!(z, "Got request message");
    let d_req = digest::d(m);
    let mut zs = z.mut_state.lock().unwrap();
    if already_handled_msg(&zs, m) {
        // possibility 1: response is cached
        z_debug!(z, "Sending cached response to client");
        return zs.replies.get(&m.c).unwrap_or(&None).clone();
    } else if zs.reqs_without_ors.contains_key(&d_req) {
        // this is a client retransmission
        start_ihatetheprimary_timer(z, &mut zs);
    }
    if !zs.pending_ors.is_empty()
        && zs.pending_ors[0].base.d_req == d_req
        && zs.pending_ors[0].base.n == (zs.n + 1) as u64
    {
        // possibility 2: we have the OR already and can execute it now
        let sor = zs.pending_ors.remove(0);
        check_and_execute_request(z, zs, &sor, sm)
    } else if is_primary(z, &zs) {
        // possibility 3: we are the primary and can order it ourselves
        order_message(z, &mut zs, m).and_then(|sor| check_and_execute_request(z, zs, &sor, sm))
    } else {
        // possibility 4: we wait until we receive the OR later
        let (tx, rx) = mpsc::channel();
        // note: this may clobber the client's previous request
        zs.reqs_without_ors.insert(d_req, tx);
        drop(zs);
        match rx.recv() {
            Ok(sor) => {
                let mut zs = z.mut_state.lock().unwrap();
                zs.reqs_without_ors.remove(&d_req);
                check_and_execute_request(z, zs, &sor, sm)
            }
            // our request may have been clobbered
            Err(_) => None,
        }
    }
}

/// Takes the given message and returns an OrderedRequestMessage to be applied.
/// Also starts a thread to send the OrderedRequestMessage to all servers.
/// Does not apply the message.
fn order_message(
    z: &ZenoState,
    zs: &mut ZenoMutState,
    m: &RequestMessage,
) -> Option<Signed<OrderedRequestMessage>> {
    let last_t: i64;

    zs.executed_reqs.entry(m.c).or_insert_with(Vec::new);
    assert!(zs.all_executed_reqs.len() == (zs.n + 1) as usize);

    if !zs.executed_reqs[&m.c].is_empty() {
        let last_req_option = zs.executed_reqs[&m.c].last();
        let last_req = last_req_option.unwrap();
        last_t = zs.all_executed_reqs[*last_req].1.base.t as i64;
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

        let sod = z.sign(OrderedRequestMessage {
            v: zs.v as u64,
            n: (zs.n + 1) as u64,
            h: h_n,
            d_req,
            i: z.me,
            s: m.s,
            nd: Vec::new(),
        });
        z.send_to_all(Message::OrderedRequest(sod.clone()));
        Some(sod)
    }
}

/// Generates a client response based on a request and orderedrequest
///
/// Assumes that we've already verified om and msg and they are correct.
fn generate_client_response(
    z: &ZenoState,
    app_response: Vec<u8>,
    om: &OrderedRequestMessage,
    msg: &RequestMessage,
) -> Message {
    let crm = ClientResponseMessage {
        response: ConcreteClientResponseMessage::SpecReply(z.sign(message::SpecReplyMessage {
            v: om.v,
            n: om.n,
            h: om.h,
            d_r: digest::d(app_response.clone()),
            c: msg.c,
            t: msg.t,
        })),
        j: z.me,
        r: app_response,
        or: om.clone(),
    };
    Message::ClientResponse(z.sign(Box::new(crm)))
}

/// Runs checks and executes the given request.
///
/// If the request checks out, this method sends it to the
/// application, waits for a response, and finally returns a message to send
/// back to the client.
fn check_and_execute_request(
    z: &ZenoState,
    mut zs: MutexGuard<ZenoMutState>,
    sor: &Signed<OrderedRequestMessage>,
    smsg: &Signed<RequestMessage>,
) -> Option<Message> {
    let or = &sor.base;
    let msg = &smsg.base;
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
        return None;
    }

    // checks done, let's execute
    let (tx, rx) = mpsc::channel();
    zs.apply_tx
        .send((ApplyMsg::Apply(msg.o.clone()), tx))
        .unwrap();

    zs.n += 1;
    z_debug!(z, "Executed request {}", zs.n);
    zs.all_executed_reqs.push((sor.clone(), smsg.clone()));
    zs.executed_reqs.entry(msg.c).or_insert_with(Vec::new);
    let n = zs.n as usize;
    zs.executed_reqs.get_mut(&msg.c).unwrap().push(n);
    assert!(zs.all_executed_reqs.len() - 1 == zs.n as usize);
    zs.h.push(history_digest);

    // if we already have a next request queued, let that thread know
    if !zs.pending_ors.is_empty() && zs.pending_ors[0].base.n == (zs.n + 1) as u64 {
        let d_req = zs.pending_ors[0].base.d_req;
        if let Some(tx_next) = zs.reqs_without_ors.remove(&d_req) {
            z_debug!(z, "Sending next request OR for {}", zs.n + 1);
            tx_next.send(zs.pending_ors.remove(0)).unwrap();
        }
    }

    // strong operation -- only commit once we have a commit certificate
    if msg.s {
        let mut commit_cert = zs
            .pending_commits
            .get(&or.d_req)
            .unwrap_or(&Vec::new())
            .into_iter()
            .filter(|commit| commit.base.or == *or)
            .cloned()
            .collect::<HashSet<Signed<CommitMessage>>>();
        zs.pending_commits.remove(&or.d_req);

        let (tx_commit, rx_commit) = mpsc::channel();
        zs.reqs_without_commits.insert(or.d_req, tx_commit);
        drop(zs);

        let commit_msg = Message::Commit(z.sign(CommitMessage {
            or: or.clone(),
            j: z.me,
        }));
        z.send_to_all(commit_msg);
        while commit_cert.len() < 2 * z.max_failures as usize {
            if let Ok(commit) = rx_commit.recv() {
                if commit.base.or == *or {
                    commit_cert.insert(commit);
                }
            } else {
                // this should only happen if the server's stopping
                return None;
            }
        }

        let mut zs1 = z.mut_state.lock().unwrap();
        zs1.reqs_without_commits.remove(&or.d_req);
        zs1.last_cc = commit_cert.into_iter().collect();
        zs1.last_cc
            .sort_by(|ref a, ref b| a.base.j.cmp(&(b.base.j)));
    } else {
        drop(zs);
    }

    let app_resp = rx.recv().unwrap();
    Some(generate_client_response(z, app_resp, &or, msg))
}

fn on_ordered_request(z: &ZenoState, sor: Signed<OrderedRequestMessage>) {
    let zs = &mut *z.mut_state.lock().unwrap();
    if sor.base.v != zs.v {
        z_debug!(z, "Ignoring OR from v:{} because v is {}", sor.base.v, zs.v);
        return;
    }
    if sor.base.i != get_primary(zs) {
        z_debug!(z, "Ignoring OR from wrong primary for this view");
        return;
    }
    if sor.base.n != (zs.n + 1) as u64 {
        send_fillhole(z, zs, sor.base.n);
    }
    stop_ihatetheprimary_timer(zs);
    process_or(zs, sor);
}

/// queues the OR message if it's not already in our queue
fn queue_or(zs: &mut ZenoMutState, sor: Signed<OrderedRequestMessage>) {
    match zs
        .pending_ors
        .binary_search_by_key(&sor.base.n, |o| o.base.n)
    {
        Ok(_) => {}
        Err(i) => {
            zs.pending_ors.insert(i, sor);
        }
    }
}

fn send_fillhole(z: &ZenoState, zs: &mut ZenoMutState, n: u64) {
    let v = zs.v;
    let zs_n = zs.n;
    let i = get_primary(zs);
    let msg = Message::FillHole(FillHoleMessage {
        v,
        n: zs_n,
        or_n: n,
        i,
    });
    z.send_to(msg, i)
}

fn process_or(zs: &mut ZenoMutState, sor: Signed<OrderedRequestMessage>) {
    if sor.base.n as i64 <= zs.n {
        return;
    }
    match zs.reqs_without_ors.remove(&sor.base.d_req) {
        Some(tx) => {
            if sor.base.n == (zs.n + 1) as u64 {
                tx.send(sor).unwrap();
            } else {
                zs.reqs_without_ors.insert(sor.base.d_req, tx);
                queue_or(zs, sor);
            }
        }
        None => {
            if !zs.pending_ors.iter().any(|pending_or| *pending_or == sor) {
                queue_or(zs, sor);
            }
        }
    }
}

fn on_commit(z: &ZenoState, scm: Signed<CommitMessage>) {
    let zs = &mut *z.mut_state.lock().unwrap();
    match zs.reqs_without_commits.get(&scm.base.or.d_req) {
        Some(tx) => {
            z_debug!(z, "Sending commit to existing thread");
            tx.send(scm).ok();
        }
        None => {
            z_debug!(z, "queueing commit");
            zs.pending_commits
                .entry(scm.base.or.d_req)
                .or_insert_with(Vec::new);
            zs.pending_commits
                .get_mut(&scm.base.or.d_req)
                .unwrap()
                .push(scm);
        }
    }
}

fn is_view_active(zs: &ZenoMutState) -> bool {
    match zs.view_state {
        ViewState::ViewActive => true,
        ViewState::ViewChanging => false,
    }
}

fn on_ihatetheprimary(z: &ZenoState, msg: &IHateThePrimaryMessage) {
    let mut zs = z.mut_state.lock().unwrap();
    if msg.v == zs.v {
        zs.ihatetheprimary_accusations.insert(msg.i);
        // switch the view from active to inactive if we've gotten enough
        if is_view_active(&zs) {
            let num_accusations = zs.ihatetheprimary_accusations.len();
            if num_accusations > z.max_failures as usize {
                zs.view_state = ViewState::ViewChanging;
                zs.v += 1;
                let o = match zs.last_cc.get(0) {
                    Some(scm) => {
                        let last_committed = scm.base.or.n as usize;
                        zs.all_executed_reqs[last_committed + 1..]
                            .iter()
                            .map(|x| x.0.clone())
                            .collect()
                    }
                    None => Vec::new(),
                };
                let msg = Message::ViewChange(z.sign(message::ViewChangeMessage {
                    v: zs.v,
                    cc: zs.last_cc.clone(),
                    o,
                    i: z.me,
                }));
                z.send_to_all(msg);
            }
        }
    }
}

fn apply_new_view(_z: &ZenoState, zs: &mut ZenoMutState, nvm: &NewViewMessage) {
    for view_change in &nvm.p {
        for orm in &view_change.o {
            process_or(zs, orm.clone());
        }
        let new_view_cc_n = match view_change.cc.first() {
            Some(cm) => cm.base.or.n as i64,
            None => -1,
        };
        let our_cc_n = match zs.last_cc.first() {
            Some(cm) => cm.base.or.n as i64,
            None => -1,
        };
        if new_view_cc_n > our_cc_n {
            zs.last_cc = view_change.cc.clone();
        }
    }
    stop_ihatetheprimary_timer(zs);
    zs.v = nvm.v;
}

/// processes a view change message
fn on_viewchange(z: &ZenoState, msg: ViewChangeMessage) {
    let zs = &mut *z.mut_state.lock().unwrap();
    if msg.v == zs.v {
        zs.view_changes.insert(msg.i, msg);
        match zs.view_state {
            ViewState::ViewActive => {
                if zs.view_changes.len() > z.max_failures as usize * 2 && is_primary(z, zs) {
                    let nvm = NewViewMessage {
                        v: zs.v,
                        p: zs.view_changes.values().cloned().collect(),
                        i: z.me,
                    };
                    apply_new_view(z, zs, &nvm);
                    let msg = Message::NewView(z.sign(nvm));
                    z.send_to_all(msg);
                }
            }
            ViewState::ViewChanging => {}
        }
    }
}

fn on_newview(z: &ZenoState, msg: &NewViewMessage) {
    let zs = &mut *z.mut_state.lock().unwrap();
    if get_primary_for_view(zs, msg.v as usize) != msg.i {
        // the sender was not the right primary for this view
        return;
    }
    if msg.p.len() < z.max_failures as usize * 2 + 1 {
        // too few viewchange messages collected by primary
        return;
    }
    apply_new_view(z, zs, &msg);
    // TODO: to support weak requests, merge all messages in p
    // with the merge protocol
}

fn on_fillhole(z: &ZenoState, fhm: &FillHoleMessage) {
    let zs = &mut *z.mut_state.lock().unwrap();
    for n in ((fhm.n + 1) as u64)..fhm.or_n {
        if let Some((orm, rm)) = zs.all_executed_reqs.get(n as usize) {
            z.send_to(Message::OrderedRequest(orm.clone()), fhm.i);
            z.send_to(Message::Request(rm.clone()), fhm.i);
        }
    }
}

pub fn verifier(m: &Message) -> bool {
    match m {
        Message::Request(srm) => srm.verify(&srm.base.c),
        Message::OrderedRequest(sorm) => sorm.verify(&sorm.base.i),
        Message::ClientResponse(scrm) => scrm.verify(&scrm.base.j),
        Message::Commit(scm) => scm.verify(&scm.base.j),
        Message::IHateThePrimary(sihtpm) => sihtpm.verify(&sihtpm.base.i),
        Message::ViewChange(svcm) => svcm.verify(&svcm.base.i),
        Message::NewView(snvm) => snvm.verify(&snvm.base.i),
        Message::FillHole(_) => true,
        _ => false,
    }
}

impl ZenoState {
    fn sign<T: Serialize>(&self, thing: T) -> Signed<T> {
        Signed::new(thing, &self.private_me)
    }

    fn halt(&mut self) {
        // cleanup all active threads/etc, if we can
        if let Ok(mut zs) = self.mut_state.lock() {
            zs.alive = false;
            zs.reqs_without_ors.clear();
            zs.reqs_without_commits.clear();
        }
    }

    fn send_to_all(&self, msg: Message) {
        self.network_channel
            .send((MessageTarget::All, msg))
            .unwrap();
    }

    fn send_to(&self, msg: Message, target: signed::Public) {
        self.network_channel
            .send((MessageTarget::One(target), msg))
            .unwrap();
    }

    fn handle_message(self, m: Message) -> Option<Message> {
        if !verifier(&m) {
            z_debug!(self, "Unable to verify message!");
            return None;
        }
        match m {
            Message::Request(srm) => {
                let reply_opt = on_request_message(&self, &srm);
                if let Some(reply) = reply_opt.clone() {
                    let mut zs = self.mut_state.lock().unwrap();
                    zs.replies.insert(srm.base.c, Some(reply));
                }
                reply_opt
            }
            Message::OrderedRequest(sorm) => {
                on_ordered_request(&self, sorm);
                None
            }
            Message::Commit(scm) => {
                z_debug!(self, "GOT COMMIT!");
                on_commit(&self, scm);
                None
            }
            Message::IHateThePrimary(sihtpm) => {
                z_debug!(self, "GOT IHTP");
                on_ihatetheprimary(&self, &sihtpm.base);
                None
            }

            Message::ViewChange(svcm) => {
                z_debug!(self, "GOT ViewChange");
                on_viewchange(&self, svcm.base);
                None
            }
            Message::NewView(snvm) => {
                z_debug!(self, "GOT NewView");
                on_newview(&self, &snvm.base);
                None
            }
            Message::FillHole(fhm) => {
                z_debug!(self, "GOT FillHole");
                on_fillhole(&self, &fhm);
                None
            }
            _ => None,
        }
    }
}

#[cfg_attr(feature = "cargo-clippy", allow(implicit_hasher))]
pub fn start_zeno(
    url: &str,
    kp: &signed::KeyPair,
    pubkeys: Vec<signed::Public>,
    pubkeys_to_url: &HashMap<signed::Public, String>,
    apply_tx: Sender<(ApplyMsg, Sender<Vec<u8>>)>,
    max_failures: u64,
) -> Zeno {
    let mut pubkeys_to_url_without_me = pubkeys_to_url.clone();
    pubkeys_to_url_without_me.remove(&kp.0);
    let (tx, rx) = mpsc::channel();
    let state = ZenoState {
        url: url.to_string(),
        me: kp.clone().0,
        private_me: kp.clone().1,
        max_failures,
        network_channel: tx,
        mut_state: Arc::new(Mutex::new(ZenoMutState {
            alive: true,
            pubkeys,
            n: -1,
            v: 0,
            h: Vec::new(),
            executed_reqs: HashMap::new(),
            replies: HashMap::new(),
            all_executed_reqs: Vec::new(),
            reqs_without_ors: HashMap::new(),
            reqs_without_commits: HashMap::new(),
            pending_ors: Vec::new(),
            pending_commits: HashMap::new(),
            apply_tx,
            last_cc: Vec::new(),
            ihatetheprimary_timer_stopper: None,
            ihatetheprimary_accusations: HashSet::new(),
            view_changes: HashMap::new(),
            view_state: ViewState::ViewActive,
            new_views: HashMap::new(),
        })),
    };
    let net = Network::new(
        &url,
        pubkeys_to_url_without_me,
        Some((ZenoState::handle_message, state.clone())),
    );
    let net1 = net.clone();
    thread::spawn(move || send_from_channel(&net1, rx));
    Zeno {
        state,
        network: net,
    }
}

#[cfg(test)]
mod tests {
    use super::start_zeno;
    use super::ApplyMsg;
    use signed;
    use std::collections::HashMap;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::sync::mpsc::Receiver;
    use std::thread;
    use std::time;
    use zeno::Zeno;
    use zeno::ZenoState;
    use zeno_client;

    use rand::thread_rng;
    use rand::Rng;

    use digest::HashDigest;

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
        evil_byzantine_thread: Option<(Vec<usize>, fn(z: ZenoState))>,
    ) -> (Vec<Zeno>, HashMap<signed::Public, String>) {
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
                &urls[i].clone(),
                &keypairs[i].clone(),
                keypairs.iter().map(|kp| kp.0).collect(),
                &pubkeys_to_urls,
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

        // Simulate faults on one of the nodes
        match evil_byzantine_thread {
            Some((faulty_servers, byzantine_function)) => {
                for faulty_server in faulty_servers {
                    let faulty_zeno = zenos.get(faulty_server).unwrap().state.clone();
                    let byzantine_function_1 = byzantine_function.clone();
                    thread::spawn(move || byzantine_function_1(faulty_zeno));
                }
            }
            None => {}
        }

        // give the servers some time to know each other
        // TODO: detect stabilization rather than sleep
        thread::sleep(time::Duration::new(2, 0));

        (zenos, pubkeys_to_urls)
    }

    #[test]
    fn test_one_client_strong_consistency() {
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![], None);

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
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![2], None);

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
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![0], None);

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
            let mut c = zeno_client::Client::new(signed::gen_keys(), pubkeys_to_urls, max_failures);
            for (i, op) in ops.into_iter().enumerate() {
                let result = c.request(op, strong);
                if let Some(e) = expected.as_ref() {
                    assert_eq!(e[i], result);
                }
            }
            tx.send(()).ok();
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
    #[should_panic]
    fn test_many_clients_strong_consistency_too_many_fails() {
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![0, 1], None);

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
        rx.recv_timeout(time::Duration::from_secs(10)).unwrap();
    }

    fn test_input_output(
        input: Vec<Vec<u8>>,
        output: Vec<Vec<u8>>,
        num_servers: usize,
        max_failures: usize,
        strong: bool,
        num_clients: usize,
    ) {
        let (_zenos, pubkeys_to_urls) =
            start_zenos::<EchoApp>(num_servers, max_failures, vec![], None);
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

    fn wait_on_all_benchmark(rxs: Vec<Receiver<()>>, timeout: time::Duration) -> time::Duration {
        let start = time::Instant::now();
        let mut average_response_time: time::Duration = time::Duration::new(0, 0);
        for rx in &rxs {
            assert_eq!(rx.recv_timeout(timeout - start.elapsed()), Ok(()));
            let elapsed = start.elapsed();
            println!("Elapsed: {:?}", elapsed);
            average_response_time += elapsed;
        }
        average_response_time / (rxs.len() as u32)
    }

    fn many_clients_strong_consistency() -> time::Duration {
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![], None);

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
        let avg = wait_on_all_benchmark(client_rxs, time::Duration::from_secs(30));

        println!("Average latency: {:?}", avg);

        let rx = do_ops_as_new_client(
            pubkeys_to_urls.clone(),
            vec![vec![0]],
            Some(vec![vec![100]]),
            1,
            true,
        );

        rx.recv_timeout(time::Duration::from_secs(30)).unwrap();

        avg
    }

    fn many_clients_strong_consistency_primary_fail() -> time::Duration {
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![0], None);

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
        let avg = wait_on_all_benchmark(client_rxs, time::Duration::from_secs(30));

        println!("Average latency: {:?}", avg);
        let rx = do_ops_as_new_client(
            pubkeys_to_urls.clone(),
            vec![vec![0]],
            Some(vec![vec![100]]),
            1,
            true,
        );
        rx.recv_timeout(time::Duration::from_secs(30)).unwrap();

        avg
    }

    fn many_clients_strong_consistency_replica_fail() -> time::Duration {
        let (_zenos, pubkeys_to_urls) = start_zenos::<CountApp>(4, 1, vec![2], None);

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
        let avg = wait_on_all_benchmark(client_rxs, time::Duration::from_secs(30));

        println!("Average latency: {:?}", avg);
        let rx = do_ops_as_new_client(
            pubkeys_to_urls.clone(),
            vec![vec![0]],
            Some(vec![vec![100]]),
            1,
            true,
        );
        rx.recv_timeout(time::Duration::from_secs(30)).unwrap();

        avg
    }

    fn duration_to_fsecs(d: &time::Duration) -> f64 {
        d.as_secs() as f64 + d.subsec_nanos() as f64 / 1e9
    }

    fn benchmark(test: fn() -> time::Duration, iters: usize) -> f64 {
        let mut times = Vec::new();
        for _ in 0..iters {
            let curr = test();
            times.push(curr);
        }
        let avg: f64 = times.iter().map(|t| duration_to_fsecs(t)).sum::<f64>() / (iters as f64);
        let stdev = (times
            .iter()
            .map(|t| duration_to_fsecs(t))
            .map(|secs| (avg - secs).powi(2))
            .sum::<f64>()
            / (iters as f64))
            .sqrt();
        println!("Avg: {:?}, stdev: {:?}", avg, stdev);
        avg
    }

    #[test]
    fn test_many_clients_strong_consistency_simple() {
        many_clients_strong_consistency();
    }

    #[test]
    fn test_many_clients_strong_consistency_primary_fail() {
        many_clients_strong_consistency_primary_fail();
    }

    #[test]
    fn test_many_clients_strong_consistency_replica_fail() {
        many_clients_strong_consistency_replica_fail();
    }

    #[test]
    #[ignore]
    fn benchmark_many_clients_strong_consistency_simple() {
        println!(
            "Final Latency: {:?}",
            benchmark(many_clients_strong_consistency, 10)
        );
    }

    #[test]
    #[ignore]
    fn benchmark_many_clients_strong_consistency_primary_fail() {
        println!(
            "Final Latency: {:?}",
            benchmark(many_clients_strong_consistency_primary_fail, 10)
        );
    }

    #[test]
    #[ignore]
    fn benchmark_many_clients_strong_consistency_replica_fail() {
        println!(
            "Final Latency: {:?}",
            benchmark(many_clients_strong_consistency_replica_fail, 10)
        );
    }

    fn run_byzantine_test(test: fn(ZenoState), faulty_servers: Vec<usize>) -> time::Duration {
        let (_zenos, pubkeys_to_urls) =
            start_zenos::<CountApp>(4, 1, vec![], Some((faulty_servers, test)));

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

        let avg = wait_on_all_benchmark(client_rxs, time::Duration::from_secs(30));

        println!("Average latency: {:?}", avg);

        let rx = do_ops_as_new_client(
            pubkeys_to_urls.clone(),
            vec![vec![0]],
            Some(vec![vec![100]]),
            1,
            true,
        );

        rx.recv_timeout(time::Duration::from_secs(30)).unwrap();

        avg
    }

    fn byzantine_hash_digest_mutilation(z: ZenoState) {
        loop {
            {
                let mut zs = z.mut_state.lock().unwrap();

                let kind_of_failure = thread_rng().gen_range(0, 3);

                let h_len = zs.h.len();
                if kind_of_failure == 0 {
                    if zs.h.len() > 0 {
                        zs.h.pop();
                    }
                } else if kind_of_failure == 1 {
                    zs.h.push([1, 2, 3, 4]);
                } else if kind_of_failure == 2 && h_len > 0 {
                    zs.h[h_len - 1] = [1, 2, 3, 4];
                }
            }
            thread::sleep(time::Duration::new(0, 100));
        }
    }

    fn byzantine_commit_mutilation(z: ZenoState) {
        loop {
            {
                let mut zs = z.mut_state.lock().unwrap();
                let pending = &mut zs.pending_commits;
                if !pending.is_empty() {
                    let random_index = thread_rng().gen_range(0, pending.len());
                    let values: Vec<HashDigest> = pending.keys().map(|k| k.clone()).collect();

                    let random_value: &HashDigest = values.get(random_index).unwrap();

                    pending.remove(random_value);
                }
            }
            thread::sleep(time::Duration::new(0, 100));
        }
    }

    fn byzantine_n_mutilation(z: ZenoState) {
        loop {
            {
                let mut zs = z.mut_state.lock().unwrap();

                zs.n = 0;
            }
            thread::sleep(time::Duration::new(0, 100));
        }
    }

    fn base_byzantine_hash_digest_mutlation_replica_simple() -> time::Duration {
        run_byzantine_test(byzantine_hash_digest_mutilation, vec![2])
    }

    #[test]
    fn test_byzantine_hash_digest_mutlation_replica_simple() {
        base_byzantine_hash_digest_mutlation_replica_simple();
    }

    #[test]
    #[ignore]
    fn benchmark_byzantine_hash_digest_mutlation_replica_simple() {
        benchmark(base_byzantine_hash_digest_mutlation_replica_simple, 10);
    }

    #[test]
    #[should_panic]
    fn test_byzantine_hash_digest_mutlation_replica_too_many() {
        run_byzantine_test(byzantine_hash_digest_mutilation, vec![1, 2]);
    }

    fn base_byzantine_commit_mutlation_replica_simple() -> time::Duration {
        run_byzantine_test(byzantine_commit_mutilation, vec![2])
    }

    #[test]
    fn test_byzantine_commit_mutlation_replica_simple() {
        base_byzantine_commit_mutlation_replica_simple();
    }

    #[test]
    #[ignore]
    fn benchmark_byzantine_commit_mutlation_replica_simple() {
        benchmark(base_byzantine_commit_mutlation_replica_simple, 10);
    }

    fn base_byzantine_n_mutlation_replica_simple() -> time::Duration {
        run_byzantine_test(byzantine_n_mutilation, vec![2])
    }

    #[test]
    fn test_byzantine_n_mutlation_replica_simple() {
        base_byzantine_n_mutlation_replica_simple();
    }

    #[test]
    #[ignore]
    fn benchmark_byzantine_n_mutlation_replica_simple() {
        benchmark(base_byzantine_n_mutlation_replica_simple, 10);
    }

    fn base_byzantine_n_mutlation_replica_primary() -> time::Duration {
        run_byzantine_test(byzantine_n_mutilation, vec![0])
    }

    #[test]
    fn test_byzantine_n_mutlation_replica_primary() {
        base_byzantine_n_mutlation_replica_primary();
    }

    #[test]
    #[ignore]
    fn benchmark_byzantine_n_mutlation_replica_primary() {
        benchmark(base_byzantine_n_mutlation_replica_primary, 10);
    }

    #[test]
    #[should_panic]
    fn test_byzantine_n_mutlation_replica_too_many() {
        run_byzantine_test(byzantine_n_mutilation, vec![1, 2]);
    }
}
