extern crate basementdb;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate clap;
extern crate serde_json;

use clap::{App, Arg, SubCommand};
use std::fs::File;

#[derive(Serialize, Deserialize, Debug)]
struct ClusterConfig {
    nodes: Vec<String>,
    f: u64,
}

fn load_cluster_config(path: &str) -> ClusterConfig {
    let file = File::open(path).expect("file not found");
    let config: ClusterConfig = serde_json::from_reader(file).expect("error while reading json");
    config
}

fn main() {
    let matches = App::new("BasementDB")
        .version("0.1")
        .author(crate_authors!())
        .subcommand(
            SubCommand::with_name("gen_keys")
                .about("Generates a public/private key pair")
                .arg(Arg::with_name("name").required(true)),
        )
        .arg(Arg::with_name("bind url").required(true).takes_value(true))
        .arg(
            Arg::with_name("private key path")
                .required(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("cluster config file")
                .required(true)
                .takes_value(true),
        )
        .get_matches();

    let cluster_config_path = matches.value_of("cluster config file").unwrap();
    let cluster_config = load_cluster_config(cluster_config_path);
    println!("{:?}", cluster_config);
}
