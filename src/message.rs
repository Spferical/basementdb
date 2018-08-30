use digest::HashDigest;
use signed;
use std::vec::Vec;

use signed::Signed;
use str_serialize::StrSerialize;

#[allow(dead_code)]
enum MessageType {
    // 4.4 Sequence Number Assignment
    Request,        // Client Request (Request)
    Or,             // OrderedRequest (Request)
    Commit,         // Commit (Request)
    ClientResponse, // Client Response (Response)
    FillHole,       // Fill Hole (Reply, in response to `Commit` or `OR` from server)

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

pub type CommitCertificate = Vec<Signed<CommitMessage>>;

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct RequestMessage {
    pub o: Vec<u8>,        // Operation to be performed
    pub t: u64,            // Timestamp assigned by the client to each request
    pub c: signed::Public, // Client public key
    pub s: bool,           // Flag indicating if this is a strong operation
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct OrderedRequestMessage {
    pub v: u64,            // Current view number
    pub n: u64,            // Highest sequence number executed
    pub h: HashDigest,     // History, a hash-chain digest of the requests
    pub d_req: HashDigest, // Digest of the current request
    pub i: signed::Public, // Primary public key
    pub s: bool,           // Flag indicating if this is a strong operation
    pub nd: Vec<u8>,       // nd is a set of non-deterministic application variables
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponseMessage {
    pub response: Signed<ReplyMessage>, // The first chunk of the response
    pub j: signed::Public,              // Replica public key
    pub r: Vec<u8>,                     // Result of the operation performed
    pub or: OrderedRequestMessage,      // OrderedRequestMessage
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct CommitMessage {
    pub or: OrderedRequestMessage, // OrderedRequestMessage
    pub j: signed::Public,         // Replica public key
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct ReplyMessage {
    pub v: u64,            // Current view number
    pub n: u64,            // Highest sequence number executed
    pub h: HashDigest,     // History, a hash-chain digest of the requests
    pub d_r: HashDigest,   // Digest of the result `r`
    pub c: signed::Public, // Client public key
    pub t: u64,            // Timestamp assigned by the client to each request
    pub s: bool,           // Whether this is a strong reply
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct FillHoleMessage {
    pub v: u64,            // Current view number
    pub n: i64,            // Highest sequence number executed
    pub or_n: u64,         // OrderedRequestMessage.n
    pub i: signed::Public, // Primary public key
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct TestMessage {
    pub c: signed::Public,
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct IHateThePrimaryMessage {
    pub v: u64,
    pub i: signed::Public,
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct ViewChangeMessage {
    pub v: u64,
    pub cc: CommitCertificate,
    pub o: Vec<Signed<OrderedRequestMessage>>,
    pub i: signed::Public,
}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct NewViewMessage {
    pub v: u64,
    pub p: Vec<ViewChangeMessage>,
    // for easy verification. but v must be checked against i later
    pub i: signed::Public,
}
#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct ViewConfirmMessage {}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct POMMessage {}
#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct PODMessage {}
#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct POAMessage {}

#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub struct CheckPointMessage {}

#[cfg_attr(feature = "cargo-clippy", allow(large_enum_variant))]
#[derive(PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Clone)]
pub enum Message {
    Request(Signed<RequestMessage>),
    OrderedRequest(Signed<OrderedRequestMessage>),
    ClientResponse(Box<ClientResponseMessage>),
    Commit(Signed<CommitMessage>),
    FillHole(FillHoleMessage),

    IHateThePrimary(Signed<IHateThePrimaryMessage>),
    ViewChange(Signed<ViewChangeMessage>),
    NewView(Signed<NewViewMessage>),
    ViewConfirm(Signed<ViewConfirmMessage>),

    POM(Signed<POMMessage>),
    POD(Signed<PODMessage>),
    POA(Signed<POAMessage>),

    CheckPoint(Signed<CheckPointMessage>),

    Dummy,
    // For debugging purposes
    Test(TestMessage),
}

impl StrSerialize<Message> for Message {}
