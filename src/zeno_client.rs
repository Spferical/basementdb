use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time;

use message;
use signed;
use tcp::Network;

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
            net: Network::new::<i32>("n/a".to_string(), pubkeys_to_url, None),
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
        let done = Arc::new(Mutex::new(false));
        let (tx, rx) = mpsc::channel();
        for rec in self.server_pubkeys.iter() {
            let tx1 = tx.clone();
            let net = self.net.clone();
            let m1 = m.clone();
            let rec1 = rec.clone();
            let done1 = done.clone();
            thread::spawn(move || loop {
                println!("Trying request...");
                if let Ok(reply) = net.send_recv(m1.clone(), rec1) {
                    println!("Request success! {:?}", reply);
                    tx1.send((rec1, reply)).ok();
                    break;
                } else if *done1.lock().unwrap() {
                    println!("Done!");
                    break;
                }
                thread::sleep(time::Duration::from_millis(100));
            });
        }

        let mut responses = HashMap::new();
        for (_target, _msg) in rx.iter() {
            let num = responses.entry(0).or_insert(0);
            *num += 1;
            if *num > 0 {
                *done.lock().unwrap() = true;
                println!("All done!");
                return;
            }
        }
        panic!("Channel closed!");
    }
}
