#![feature(plugin, use_extern_macros)]
#![plugin(tarpc_plugins)]

#[macro_use]
extern crate tarpc;

use tarpc::sync::{client, server};
use tarpc::sync::client::ClientExt;
use tarpc::util::Never;
use std::sync::mpsc;
use std::thread;

service! {
    rpc hello(name: String) -> String;
}

#[derive(Clone)]
struct HelloServer;

impl SyncService for HelloServer {
    fn hello(&self, name: String) -> Result<String, Never> {
        Ok(format!("Hello, {}!", name))
    }
}

fn main() {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let handle = HelloServer.listen("localhost:10000",
            server::Options::default()).unwrap();
        tx.send(handle.addr()).unwrap();
        handle.run();
    });
    let addr = rx.recv().unwrap();
    let client = SyncClient::connect(addr, client::Options::default()).unwrap();
    println!("{}", client.hello("world".to_string()).unwrap());
}
