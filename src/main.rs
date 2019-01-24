extern crate basementdb;
extern crate sodiumoxide;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate clap;

use basementdb::signed::Public;
use clap::{App, AppSettings, Arg, SubCommand};
use sodiumoxide::crypto::sign::SecretKey as Private;

extern crate base64;
extern crate serde;
extern crate serde_json;

use std::fs;
use std::fs::File;

#[derive(Serialize, Deserialize, Debug)]
pub struct ClusterConfigNode {
    url: String,
    pubkey_path: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClusterConfig {
    nodes: Vec<ClusterConfigNode>,
    f: u64,
}

pub fn load_cluster_config(path: &str) -> ClusterConfig {
    let file = File::open(path).expect("file not found");
    let config: ClusterConfig = serde_json::from_reader(file).expect("error while reading json");
    config
}

pub fn sample_config() -> ClusterConfig {
    let urls: Vec<_> = (10001..=10004)
        .map(|p| format!("localhost:{}", p))
        .collect();
    ClusterConfig {
        nodes: (0..4)
            .map(|i| ClusterConfigNode {
                url: urls[i].clone(),
                pubkey_path: "./".to_owned() + &urls[i] + ".pub",
            })
            .collect(),
        f: 1,
    }
}

fn main() {
    let matches = App::new("BasementDB")
        .setting(AppSettings::ArgRequiredElseHelp)
        .version("0.1")
        .author(crate_authors!())
        .subcommand(SubCommand::with_name("gen_keys").about("Generates a public/private key pair"))
        .subcommand(SubCommand::with_name("gen_config").about("Generates an example config file"))
        .subcommand(
            SubCommand::with_name("run")
                .about("Runs a node of BasementDB")
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
                ),
        )
        .get_matches();

    if matches.is_present("gen_keys") {
        let (pubkey, privkey) = basementdb::signed::gen_keys();
        println!(
            "pub:{}\npriv:{}",
            base64::encode(&pubkey[..]),
            base64::encode(&privkey[..])
        );
        return;
    } else if matches.is_present("gen_config") {
        let config = sample_config();
        println!("{}", serde_json::to_string(&config).unwrap());
        let keys = (0..4).map(|_| basementdb::signed::gen_keys());
        let node_urls = config.nodes.iter().map(|node| &node.url);
        for (key, url) in keys.zip(node_urls) {
            fs::write(format!("{}.pub", url), base64::encode(&key.0)).unwrap();
            let Private(ref skbytes) = key.1;
            fs::write(format!("{}.priv", url), base64::encode(&skbytes.to_vec())).unwrap();
        }
    } else if matches.is_present("run") {
        let cluster_config_path = matches.value_of("cluster config file").unwrap();
        let cluster_config = load_cluster_config(cluster_config_path);
        println!("{:?}", cluster_config);
    }
}
