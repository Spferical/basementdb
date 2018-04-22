use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use tarpc::sync::{client, server};
use tarpc::util::Never;

struct Zeno;

service! {
    rpc echo(text: String) -> String;
}

#[derive(Clone)]
pub struct ZenoService {
    url: String,
    zeno: Arc<Mutex<Zeno>>,
}

impl SyncService for ZenoService {
    fn echo(&self, text: String) -> Result<String, Never> {
        Ok(text)
    }
}

impl ZenoService {
    pub fn new(url: String) -> ZenoService {
        ZenoService {
            url: url,
            zeno: Arc::new(Mutex::new(Zeno {})),
        }
    }

    /// starts the server threads and blocks until it is listening
    pub fn start(&self) -> SocketAddr {
        let (tx, rx) = mpsc::channel();
        let clone = self.clone();
        thread::spawn(move || {
            let url = clone.url.clone();
            let handle = clone.listen(url, server::Options::default()).unwrap();
            tx.send(handle.addr()).unwrap();
            handle.run()
        });
        rx.recv().unwrap()
    }
}
