use std::collections::HashMap;

use message;
use tcp::Network;
use signed;

pub struct Client {
    net: Network,
    seqno: u64,
    keypair: signed::KeyPair,
    server_pubkeys: Vec<signed::Public>,
}

impl Client {
    pub fn new(
        keypair: signed::KeyPair,
        pubkeys_to_url: HashMap<signed::Public, String>,
    ) -> Client {
        let pubkeys = pubkeys_to_url.keys().map(|p| *p).collect();
        Client {
            net: Network::new::<i32>("".to_string(), pubkeys_to_url, None),
            seqno: 0,
            keypair: keypair,
            server_pubkeys: pubkeys,
        }
    }

    pub fn request(&mut self, op: Vec<u8>, strong: bool) {
        let rm = message::RequestMessage {
            o: op,
            t: self.seqno,
            c: self.keypair.0,
            s: strong,
        };
        let um = message::UnsignedMessage::Request(rm);
        let s = signed::Signed::new(um, &self.keypair.1);
        let m = message::Message::Signed(s);
        for rec in self.server_pubkeys.iter() {
            self.net.send(m.clone(), rec.clone());
        }
    }
}
