extern crate futures;
extern crate tarpc;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use zeno::futures::Future;

use tarpc::future::client::ClientExt;
use tarpc::future::{client, server};
use tarpc::util::FirstSocketAddr;
use tarpc::util::Never;
use tokio_core::reactor;

struct Zeno {
    servers: HashMap<String, Option<SyncClient>>,
}

service! {
    rpc echo(text: String) -> String;
}

#[derive(Clone)]
pub struct ZenoService {
    url: String,
    zeno: Arc<Mutex<Zeno>>,
}

impl FutureService for ZenoService {
    type EchoFut = Result<String, Never>;

    fn echo(&self, text: String) -> Self::EchoFut {
        Ok(text)
    }
}

impl ZenoService {
    pub fn new(url: String, server_urls: Vec<String>) -> ZenoService {
        let servers: HashMap<_, _> = server_urls
            .iter()
            .filter(|u| **u != url)
            .map(|u| (u.clone(), None))
            .collect();
        ZenoService {
            url: url,
            zeno: Arc::new(Mutex::new(Zeno { servers: servers })),
        }
    }

    /// starts the server and blocks until the server stops
    pub fn run(&self) {
        let mut reactor = reactor::Core::new().unwrap();
        let clone = self.clone();
        let url = clone.url.clone();
        let (handle, server) = clone
            .listen(
                url.first_socket_addr(),
                &reactor.handle(),
                server::Options::default(),
            )
            .unwrap();
        reactor.handle().spawn(server);
        let options = client::Options::default().handle(reactor.handle());
        reactor
            .run(
                FutureClient::connect(handle.addr(), options)
                    .map_err(tarpc::Error::from)
                    .and_then(|client| client.echo("Hello, world!".to_string()))
                    .map(|resp| println!("{}", resp)),
            )
            .unwrap();
    }
}
