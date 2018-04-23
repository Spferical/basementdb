use digest::{HashChain, HashDigest};
use serde::{Deserialize, Serialize};
use signed;
use std::vec::Vec;

enum MessageType {
    // 4.4 Sequence Number Assignment
    Request,   // Client Request (Request)
    OR,        // OrderedRequest (Request)
    SpecReply, // SpecReply (Reply, in response to `Request` from client)
    Commit,    // Commit (Request)
    Reply,     // Reply (Reply, in response to client request)
    FillHole,  // Fill Hole (Reply, in response to `Commit` or `OR` from server)

    // 4.5 View Change Protocol
    IHateThePrimary, // IHateThePrimary (Request)
    ViewChange,      // ViewChange (Request)
    NewView,         // NewView (Request)
    ViewConfirm,     // ViewConfirm (Request)

    // 4.6-4.7 Detecting and Merging Concurrent Histories
    POMMsg, // Proof of Misbehavior (Request)
    PODMsg, // Proof of Divergence (Request)
    POAMsg, // Proof of Absence (Request)

    // 4.8 Garbage Collection
    CheckPoint, // CheckPoint (Request)
}

#[derive(Serialize, Deserialize, Debug)]
struct RequestMessage {
    o: Vec<u8>,        // Operation to be performed
    t: usize,          // Timestamp assigned by the client to each request
    c: signed::Public, // Client public key
    s: bool,           // Flag indicating if this is a strong operation
}

#[derive(Serialize, Deserialize, Debug)]
struct OrderedRequestMessage {
    v: usize,          // Current view number
    n: usize,          // Highest sequence number executed
    h: HashChain,      // History, a hash-chain digest of the requests
    d_req: HashDigest, // Digest of the current request
    i: signed::Public, // Primary public key
    s: bool,           // Flag indicating if this is a strong operation
    ND: Vec<u8>,       // ND is a set of non-deterministic application variables
}

#[derive(Serialize, Deserialize, Debug)]
struct ClientResponseMessage {
    response: ConcreteClientResponseMessage, // The first chunk of the response
    j: signed::Public,                       // Replica public key
    r: Vec<u8>,                              // Result of the operation performed
    OR: OrderedRequestMessage,               // OrderedRequestMessage
}

#[derive(Serialize, Deserialize, Debug)]
enum ConcreteClientResponseMessage {
    SpecReply(signed::Signed<SpecReplyMessage>),
    Reply(signed::Signed<ReplyMessage>),
}

#[derive(Serialize, Deserialize, Debug)]
struct SpecReplyMessage {
    v: usize,          // Current view number
    n: usize,          // Highest sequence number executed
    h: HashChain,      // History, a hash-chain digest of the requests
    d_r: HashDigest,   // Digest of the result `r`
    c: signed::Public, // Client public key
    t: usize,          // Timestamp assigned by the client to each request
}

#[derive(Serialize, Deserialize, Debug)]
struct CommitMessage {
    OR: OrderedRequestMessage, // OrderedRequestMessage
    j: signed::Public,         // Replica public key
}

#[derive(Serialize, Deserialize, Debug)]
struct ReplyMessage {
    v: usize,          // Current view number
    n: usize,          // Highest sequence number executed
    h: HashChain,      // History, a hash-chain digest of the requests
    d_r: HashDigest,   // Digest of the result `r`
    c: signed::Public, // Client public key
    t: usize,          // Timestamp assigned by the client to each request
}

#[derive(Serialize, Deserialize, Debug)]
struct FillHoleMessage {
    v: usize,          // Current view number
    n: usize,          // Highest sequence number executed
    OR_n: usize,       // OrderedRequestMessage.n
    i: signed::Public, // Primary public key
}

#[derive(Serialize, Deserialize, Debug)]
struct IHateThePrimaryMessage {}
#[derive(Serialize, Deserialize, Debug)]
struct ViewChangeMessage {}
#[derive(Serialize, Deserialize, Debug)]
struct NewViewMessage {}
#[derive(Serialize, Deserialize, Debug)]
struct ViewConfirmMessage {}

#[derive(Serialize, Deserialize, Debug)]
struct POMMessage {}
#[derive(Serialize, Deserialize, Debug)]
struct PODMessage {}
#[derive(Serialize, Deserialize, Debug)]
struct POAMessage {}

#[derive(Serialize, Deserialize, Debug)]
struct CheckPointMessage {}

#[derive(Serialize, Deserialize, Debug)]
enum Message {
    Request(RequestMessage),
    OrderedRequest(OrderedRequestMessage),
    ClientResponse(ClientResponseMessage),
    Commit(CommitMessage),
    FillHole(FillHoleMessage),

    IHateThePrimary(IHateThePrimaryMessage),
    ViewChange(ViewChangeMessage),
    NewView(NewViewMessage),
    ViewConfirm(ViewConfirmMessage),

    POM(POMMessage),
    POD(PODMessage),
    POA(POAMessage),

    CheckPoint(CheckPointMessage),
}
