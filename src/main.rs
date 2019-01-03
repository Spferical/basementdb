extern crate basementdb;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate clap;

use clap::{App, Arg, SubCommand};
use basementdb::signed::Public;

extern crate serde;
extern crate serde_json;
extern crate base64;

use serde::{Deserialize, Deserializer, Serializer};
use std::fs::File;

fn as_base64<T, S>(buf: &T, serializer: S) -> Result<S::Ok, S::Error>
    where T: AsRef<[u8]>,
          S: Serializer
{
    serializer.serialize_str(&base64::encode(buf.as_ref()))
}

fn from_base64<'d, D>(deserializer: D) -> Result<Public, D::Error>
    where D: Deserializer<'d>
{
    use serde::de::Error;
    String::deserialize(deserializer)
        .and_then(|string| base64::decode(&string).map_err(|err| Error::custom(err.to_string())))
        .map(|bytes| Public::from_slice(&bytes))
        .and_then(|opt| opt.ok_or_else(|| Error::custom("failed to deserialize public key")))
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ClusterConfigNode {
    url: String,
    #[serde(serialize_with = "as_base64", deserialize_with = "from_base64")]
    pubkey: Public,
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
    let keys: Vec<_> = (0..4).map(|_| basementdb::signed::gen_keys()).collect();
    let urls: Vec<_> = (10001..=10004)
        .map(|p| format!("localhost:{}", p))
        .collect();
    ClusterConfig {
        nodes: (0..4)
            .map(|i| ClusterConfigNode {
                url: urls[i].clone(),
                pubkey: keys[i].0,
            })
            .collect(),
        f: 1,
    }
}

fn main() {
    let matches = App::new("BasementDB")
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
        println!("pub:{}\npriv:{}", base64::encode(&pubkey[..]),
                base64::encode(&privkey[..]));
        return;
    } else if matches.is_present("gen_config") {
        let config = sample_config();
        println!("{}", serde_json::to_string(&config).unwrap());
    } else if matches.is_present("run") {
        let cluster_config_path = matches.value_of("cluster config file").unwrap();
        let cluster_config = load_cluster_config(cluster_config_path);
        println!("{:?}", cluster_config);
    }
}
