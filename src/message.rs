use serde::Serialize;
use std::vec::Vec;

type HashDigest = [u64; 4];
type HashChain = Vec<DigestHash>;

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

struct RequestMessage {
    o: Vec<u8>, // Operation to be performed
    t: usize,   // Timestamp assigned by the client to each request
    c: u64,     // TODO; public key ID or whatever
    s: bool,    // Flag indicating if this is a strong operation
}

struct OrderedRequestMessage {
    v: usize,          // Current view number
    n: usize,          // Highest sequence number executed
    h: HashChain,      // History, a hash-chain digest of the requests
    d_req: HashDigest, // Digest of the current request
    i: u64,            // TODO; public key ID or whatever
    s: bool,           // Flag indicating if this is a strong operation
    ND: Vec<u8>,       // ND is a set of non-deterministic application variables
}

struct SpecReplyMessage {}
struct CommitMessage {}
struct ReplyMessage {}
struct FillHoleMessage {}

struct IHateThePrimaryMessage {}
struct ViewChangeMessage {}
struct NewViewMessage {}
struct ViewConfirmMessage {}

struct POMMessage {}
struct PODMessage {}
struct POAMessage {}

struct CheckPointMessage {}

enum Message {
    Request(RequestMessage),
    OrderedRequest(OrderedRequestMessage),
    SpecReply(SpecReplyMessage),
    Commit(CommitMessage),
    Reply(ReplyMessage),
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
