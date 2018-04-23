extern crate basementdb;
extern crate config;
extern crate tarpc;

use basementdb::zeno;
use std::env;
use tarpc::sync::client;
use tarpc::sync::client::ClientExt;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        println!("USAGE: {} URL", args[0]);
        return;
    }
    let our_url = args[1].clone();

    let mut settings = config::Config::default();
    settings.merge(config::File::with_name("config")).unwrap();
    settings
        .merge(config::Environment::with_prefix("BDB"))
        .unwrap();
    let debug = settings.get_bool("debug").unwrap();
    println!("debug: {:?}", debug);
    let servers = settings.get::<Vec<String>>("servers").unwrap();
    println!("servers: {:?}", servers);

    let z = zeno::ZenoService::new(our_url, servers);
    z.run();
}
