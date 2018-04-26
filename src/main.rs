extern crate basementdb;
extern crate config;

use std::env;

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        println!("USAGE: {} URL", args[0]);
        return;
    }
    let _url = args[1].clone();

    let mut settings = config::Config::default();
    settings.merge(config::File::with_name("config")).unwrap();
    settings
        .merge(config::Environment::with_prefix("BDB"))
        .unwrap();
    let debug = settings.get_bool("debug").unwrap();
    println!("debug: {:?}", debug);
    let servers = settings.get::<Vec<String>>("servers").unwrap();
    println!("servers: {:?}", servers);

    // TODO: read private and public key from path in config
    // and start the server

}
