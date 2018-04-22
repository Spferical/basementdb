#![feature(plugin, use_extern_macros)]
#![plugin(tarpc_plugins)]

#[macro_use]
extern crate tarpc;
extern crate config;

use tarpc::sync::{client, server};
use tarpc::sync::client::ClientExt;
use tarpc::util::Never;
use std::sync::mpsc;
use std::thread;
use std::env;

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
    let args : Vec<_> = env::args().collect();
    if args.len() != 2 {
        println!("USAGE: {} URL", args[0]);
        return;
    }
    let our_url = args[1].clone();

    let mut settings = config::Config::default();
    settings.merge(config::File::with_name("config")).unwrap();
    settings.merge(config::Environment::with_prefix("BDB")).unwrap();
    let debug = settings.get_bool("debug").unwrap();
    println!("debug: {:?}", debug);
    let servers = settings.get::<Vec<String>>("servers").unwrap();
    println!("servers: {:?}", servers);

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let handle = HelloServer.listen(our_url,
            server::Options::default()).unwrap();
        tx.send(handle.addr()).unwrap();
        handle.run();
    });
    let addr = rx.recv().unwrap();
    let client = SyncClient::connect(addr, client::Options::default()).unwrap();
    println!("{}", client.hello("world".to_string()).unwrap());
}
