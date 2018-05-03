use std::collections::HashMap;

use message;
use signed;
use tcp::Network;

pub struct Client {
    net: Network,
    seqno: u64,
    keypair: signed::KeyPair,
    server_pubkeys: Vec<signed::Public>,
    max_failures: u64,
}

impl Client {
    pub fn new(
        keypair: signed::KeyPair,
        pubkeys_to_url: HashMap<signed::Public, String>,
        max_failures: u64,
    ) -> Client {
        let pubkeys = pubkeys_to_url.keys().map(|p| *p).collect();
        Client {
            net: Network::new::<i32>("n/a".to_string(), pubkeys_to_url, None),
            seqno: 0,
            keypair: keypair,
            server_pubkeys: pubkeys,
            max_failures: max_failures,
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
        let mut responses = HashMap::new();
        loop {
            for (_target, _msg) in self.net.send_to_all_and_recv(m.clone()).recv() {
                println!("Client got msg {:?}", _msg);
                let num = responses.entry(0).or_insert(0);
                *num += 1;
                if *num > self.max_failures {
                    return;
                }
            }
        }
    }
}
