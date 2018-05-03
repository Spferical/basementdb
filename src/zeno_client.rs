use std::collections::HashMap;

use message;
use message::UnsignedMessage;
use signed;
use tcp::Network;
use zeno::Zeno;

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

    fn get_data(&self, m: message::Message) -> Option<Vec<u8>> {
        match m {
            message::Message::Signed(sm) => {
                if let Some(u) = Zeno::verifier(sm) {
                    match u {
                        UnsignedMessage::ClientResponse(crm) => Some(crm.r),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn request(&mut self, op: Vec<u8>, strong: bool) -> Vec<u8> {
        let rm = message::RequestMessage {
            o: op,
            t: self.seqno,
            c: self.keypair.0,
            s: strong,
        };
        self.seqno += 1;
        let um = UnsignedMessage::Request(rm);
        let s = signed::Signed::new(um, &self.keypair.1);
        let m = message::Message::Signed(s);
        let mut responses = HashMap::new();
        loop {
            for (_target, resp) in self.net.send_to_all_and_recv(m.clone()).recv() {
                if let Ok(resp_msg) = resp {
                    match self.get_data(resp_msg) {
                        Some(data) => {
                            let num = responses.entry(data.clone()).or_insert(0);
                            *num += 1;
                            if *num > self.max_failures {
                                // we got a weak quorum of replies
                                return data;
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }
}
