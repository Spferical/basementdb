use bincode::{deserialize, serialize};
use data_encoding::BASE64;
use digest::{HashChain, HashDigest};
use serde::{Deserialize, Serialize};
use signed;
use std::io;
use std::vec::Vec;

use str_serialize::StrSerialize;

enum MessageType {
    // 4.4 Sequence Number Assignment
    Request,        // Client Request (Request)
    OR,             // OrderedRequest (Request)
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

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestMessage {
    pub o: Vec<u8>,        // Operation to be performed
    pub t: usize,          // Timestamp assigned by the client to each request
    pub c: signed::Public, // Client public key
    pub s: bool,           // Flag indicating if this is a strong operation
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OrderedRequestMessage {
    pub v: usize,          // Current view number
    pub n: usize,          // Highest sequence number executed
    pub h: HashChain,      // History, a hash-chain digest of the requests
    pub d_req: HashDigest, // Digest of the current request
    pub i: signed::Public, // Primary public key
    pub s: bool,           // Flag indicating if this is a strong operation
    pub ND: Vec<u8>,       // ND is a set of non-deterministic application variables
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientResponseMessage {
    pub response: ConcreteClientResponseMessage, // The first chunk of the response
    pub j: signed::Public,                       // Replica public key
    pub r: Vec<u8>,                              // Result of the operation performed
    pub OR: OrderedRequestMessage,               // OrderedRequestMessage
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ConcreteClientResponseMessage {
    SpecReply(signed::Signed<SpecReplyMessage>),
    Reply(signed::Signed<ReplyMessage>),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SpecReplyMessage {
    pub v: usize,          // Current view number
    pub n: usize,          // Highest sequence number executed
    pub h: HashChain,      // History, a hash-chain digest of the requests
    pub d_r: HashDigest,   // Digest of the result `r`
    pub c: signed::Public, // Client public key
    pub t: usize,          // Timestamp assigned by the client to each request
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CommitMessage {
    pub OR: OrderedRequestMessage, // OrderedRequestMessage
    pub j: signed::Public,         // Replica public key
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReplyMessage {
    pub v: usize,          // Current view number
    pub n: usize,          // Highest sequence number executed
    pub h: HashChain,      // History, a hash-chain digest of the requests
    pub d_r: HashDigest,   // Digest of the result `r`
    pub c: signed::Public, // Client public key
    pub t: usize,          // Timestamp assigned by the client to each request
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FillHoleMessage {
    pub v: usize,          // Current view number
    pub n: usize,          // Highest sequence number executed
    pub OR_n: usize,       // OrderedRequestMessage.n
    pub i: signed::Public, // Primary public key
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IHateThePrimaryMessage {}
#[derive(Serialize, Deserialize, Debug)]
pub struct ViewChangeMessage {}
#[derive(Serialize, Deserialize, Debug)]
pub struct NewViewMessage {}
#[derive(Serialize, Deserialize, Debug)]
pub struct ViewConfirmMessage {}

#[derive(Serialize, Deserialize, Debug)]
pub struct POMMessage {}
#[derive(Serialize, Deserialize, Debug)]
pub struct PODMessage {}
#[derive(Serialize, Deserialize, Debug)]
pub struct POAMessage {}

#[derive(Serialize, Deserialize, Debug)]
pub struct CheckPointMessage {}

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
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

impl StrSerialize<Message> for Message {}
