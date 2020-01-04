use bufstream::BufStream;
use std::collections::HashMap;
use std::io;
use std::io::BufRead;
use std::io::Write;
use std::marker::Send;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::message::Message;
use crate::signed;
use crate::str_serialize::StrSerialize;

const MAX_BUF_SIZE: usize = 1_048_576;
const READ_TIMEOUT: u64 = 2;
const WRITE_TIMEOUT: u64 = 30;
const CONNECT_TIMEOUT: u64 = 30;

#[derive(Clone)]
pub enum TCPServerCommand {
    Halt,
}

pub fn read_string_from_socket(sock: &mut BufStream<TcpStream>) -> Result<String, io::Error> {
    let mut buf: Vec<u8> = Vec::new();

    loop {
        match sock.read_until(0, &mut buf) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => {
                println!("Unable to read from TCP stream: {:?}", err);
                return Err(err);
            }
            Ok(num_bytes) => {
                if num_bytes == 0 {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Read timed out"));
                } else {
                    // remove the null byte
                    let l = buf.len();
                    buf.remove(l - 1);
                    break;
                }
            }
        }
    }

    match str::from_utf8(&buf) {
        Err(e) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Read invalid string from socket {:?}!", e),
        )),
        Ok(v) => Ok(v.to_string()),
    }
}

pub fn write_string_on_socket<T: Write>(mut sock: T, s: String) -> Result<(), io::Error> {
    let mut byte_arr = s.into_bytes();
    byte_arr.push(0);
    let num_bytes: usize = sock.write(&byte_arr)?;

    if num_bytes == 0 {
        return Err(io::Error::new(io::ErrorKind::WriteZero, "Write timed out"));
    }

    sock.flush()?;

    Ok(())
}

type ServerCallback<T> = (fn(T, Message) -> Option<Message>, T);

fn invoke<T: Clone>(callback: ServerCallback<T>, message: Message) -> Option<Message> {
    callback.0(callback.1, message)
}

pub fn start_server<T: 'static + Send + Clone>(
    net: &Network,
    receiver: &Receiver<TCPServerCommand>,
    callback: &ServerCallback<T>,
) {
    println!("Server running at {}", net.my_ip_and_port);
    let ip_and_port = net.my_ip_and_port.clone();
    let addr: SocketAddr = ip_and_port.to_socket_addrs().unwrap().next().unwrap();
    let listener = TcpListener::bind(addr).unwrap();
    let alive = Arc::new(AtomicBool::new(true));

    for stream_result in listener.incoming() {
        match stream_result {
            Err(err) => eprintln!("Err {:?}", err),
            Ok(stream) => {
                let callback1 = callback.clone();
                let alive1 = alive.clone();
                let bufstream = BufStream::with_capacities(MAX_BUF_SIZE, MAX_BUF_SIZE, stream);
                thread::spawn(move || handle_reader(bufstream, &callback1, &alive1));
            }
        };

        match receiver.try_recv() {
            Err(_) => continue,
            Ok(cmd) => match cmd {
                TCPServerCommand::Halt => {
                    alive.store(false, Ordering::Relaxed);
                    return;
                }
            },
        }
    }
}

fn handle_reader<T: Clone>(
    mut client: BufStream<TcpStream>,
    callback: &ServerCallback<T>,
    alive: &AtomicBool,
) -> Result<(), io::Error> {
    loop {
        let v = read_string_from_socket(&mut client)?;

        // Lets check if we're still alive...
        {
            if !alive.load(Ordering::Relaxed) {
                return Ok(());
            }
        }

        let potential_response = invoke(callback.clone(), Message::str_deserialize(&v)?);
        match potential_response {
            None => {}
            Some(resp) => write_string_on_socket(&mut client, Message::str_serialize(&resp)?)?,
        };
    }
}

#[derive(Debug)]
pub struct TCPClient {
    ip_and_port: String,
    stream: Option<BufStream<TcpStream>>,
}

pub fn connect_to_server(ip_and_port: String) -> TCPClient {
    let sock_addr: SocketAddr = ip_and_port.to_socket_addrs().unwrap().next().unwrap();
    let s = TcpStream::connect_timeout(&sock_addr, Duration::new(CONNECT_TIMEOUT, 0));
    let stream = match s {
        Ok(tcp_stream) => {
            tcp_stream
                .set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))
                .ok();
            tcp_stream
                .set_read_timeout(Some(Duration::new(READ_TIMEOUT, 0)))
                .ok();
            Some(BufStream::with_capacities(
                MAX_BUF_SIZE,
                MAX_BUF_SIZE,
                tcp_stream,
            ))
        }
        Err(e) => {
            eprintln!("Failed to connect to {}: {:?}", ip_and_port, e);
            None
        }
    };
    TCPClient {
        ip_and_port,
        stream,
    }
}

fn try_connecting_to_everyone(
    h: HashMap<signed::Public, String>,
) -> HashMap<signed::Public, Arc<Mutex<TCPClient>>> {
    h.into_iter()
        .map(|(p, o)| (p, connect_to_server(o)))
        .map(|(p, o)| (p, Arc::new(Mutex::new(o))))
        .collect()
}

fn retry_dead_connections(
    p: &Mutex<HashMap<signed::Public, Arc<Mutex<TCPClient>>>>,
    alive: &AtomicBool,
) {
    loop {
        {
            if !alive.load(Ordering::Relaxed) {
                return;
            }
        }

        thread::sleep(Duration::new(1, 0));

        let retries: HashMap<signed::Public, String>;

        {
            let conns = &*p.lock().unwrap();
            retries = conns
                .iter()
                .filter(|(_, v)| v.lock().unwrap().stream.is_none())
                .map(|(p, v)| (*p, v.lock().unwrap().ip_and_port.clone()))
                .collect();
        }

        let updated = try_connecting_to_everyone(retries);

        {
            let conns = &mut *p.lock().unwrap();
            conns.extend(updated);
        }
    }
}

#[derive(Clone)]
pub struct Network {
    peer_send_clients: Arc<Mutex<HashMap<signed::Public, Arc<Mutex<TCPClient>>>>>,
    server_channel: Sender<TCPServerCommand>,
    alive_state: Arc<AtomicBool>,
    my_ip_and_port: String,
    server_threads: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

impl Network {
    pub fn new<T: 'static + Send + Clone>(
        my_ip: &str,
        public_key_to_ip_map: HashMap<signed::Public, String>,
        receive_callback: Option<ServerCallback<T>>,
    ) -> Network {
        let peer_send_clients = try_connecting_to_everyone(public_key_to_ip_map);
        let (tx, rx): (Sender<TCPServerCommand>, Receiver<TCPServerCommand>) = mpsc::channel();
        let psc = Arc::new(Mutex::new(peer_send_clients));
        let psc1 = psc.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let alive1 = alive.clone();
        let threads = Arc::new(Mutex::new(Vec::new()));
        let net = Network {
            peer_send_clients: psc,
            server_channel: tx,
            alive_state: alive,
            my_ip_and_port: my_ip.to_string(),
            server_threads: threads,
        };
        let net1 = net.clone();

        {
            let mut threads = net.server_threads.lock().unwrap();

            if let Some(rx_callback) = receive_callback {
                println!("{}: Starting server!", net1.my_ip_and_port.clone());
                threads.push(thread::spawn(move || {
                    start_server(&net1, &rx, &rx_callback)
                }));
            }
            threads.push(thread::spawn(move || {
                retry_dead_connections(&psc1, &alive1)
            }));
        }

        net
    }

    fn _send(m: &Message, client_raw: &mut TCPClient) -> Result<(), io::Error> {
        match &mut client_raw.stream {
            Some(stream) => {
                let s: String = Message::str_serialize(m)?;

                write_string_on_socket(stream, s)
            }
            _ => Err(io::Error::new(io::ErrorKind::NotConnected, "Not connected")),
        }
    }

    fn _recv(client_raw: &mut TCPClient) -> Result<Message, io::Error> {
        match &mut client_raw.stream.as_mut() {
            Some(stream) => Message::str_deserialize(&read_string_from_socket(stream)?),
            _ => Err(io::Error::new(io::ErrorKind::NotConnected, "Not connected")),
        }
    }

    pub fn send_recv(&self, m: &Message, recipient: signed::Public) -> Result<Message, io::Error> {
        let mut psc = self.peer_send_clients.lock().unwrap();
        if let Some(client_raw_lock) = psc.get_mut(&recipient).cloned() {
            drop(psc);
            let client_raw = &mut client_raw_lock.lock().unwrap();
            let send_result = Network::_send(&m, client_raw);

            match send_result {
                Ok(()) => {}
                Err(e) => {
                    client_raw.stream = None;
                    return Err(e);
                }
            }

            let recv_result = Network::_recv(client_raw);
            match recv_result {
                Ok(a) => Ok(a),
                Err(e) => {
                    client_raw.stream = None;
                    Err(e)
                }
            }
        } else {
            Err(io::Error::new(io::ErrorKind::NotConnected, "Not connected"))
        }
    }

    pub fn send(&self, m: &Message, recipient: &signed::Public) -> Result<(), io::Error> {
        let mut psc = self.peer_send_clients.lock().unwrap();
        if let Some(client_raw_lock) = psc.get_mut(recipient).cloned() {
            drop(psc);
            let client_raw = &mut client_raw_lock.lock().unwrap();
            let result = Network::_send(&m, client_raw);
            match result {
                Ok(()) => Ok(()),
                Err(e) => {
                    client_raw.stream = None;
                    Err(e)
                }
            }
        } else {
            Err(io::Error::new(io::ErrorKind::NotConnected, "Not connected"))
        }
    }

    pub fn send_to_all(&self, m: &Message) -> HashMap<signed::Public, Result<(), io::Error>> {
        let psc = self.peer_send_clients.lock().unwrap();
        let mut results = HashMap::new();
        type SendResult = (signed::Public, Result<(), io::Error>);
        let (tx, rx): (Sender<SendResult>, Receiver<SendResult>) = mpsc::channel();

        // We need to be convinced that our operations on psc's TCPClients
        // will not outlive psc's current binding. scoped_threadpool helps
        // with that.
        for kv in psc.iter() {
            let (pub_key, tcp_client_lock) = (*kv.0, kv.1.clone());
            let m1 = m.clone();
            let tx1 = tx.clone();
            let my_ip_and_port = self.my_ip_and_port.clone();

            // Send on this channel to signify that the operation is over.
            // We can't just mutate results directly because that would mean
            // multiple mutable borrows.
            thread::spawn(move || {
                let tcp_client = &mut tcp_client_lock.lock().unwrap();
                if my_ip_and_port != tcp_client.ip_and_port {
                    let result = Network::_send(&m1, tcp_client);
                    let unwrapped = match result {
                        Ok(()) => Ok(()),
                        Err(e) => {
                            tcp_client.stream = None;
                            Err(e)
                        }
                    };

                    tx1.send((pub_key, unwrapped)).unwrap();
                }
            });
        }

        drop(psc);
        drop(tx);

        // Wait for every TCPClient to send a message to everyone
        for msg in rx.iter() {
            results.insert(msg.0, msg.1);
        }
        results
    }

    pub fn send_to_all_and_recv(
        &self,
        m: &Message,
    ) -> Receiver<(signed::Public, Result<Message, io::Error>)> {
        let psc = self.peer_send_clients.lock().unwrap();
        type SendResult = (signed::Public, Result<Message, io::Error>);
        let (tx, rx): (Sender<SendResult>, Receiver<SendResult>) = mpsc::channel();

        for kv in psc.iter() {
            let (pub_key, tcp_client_lock) = (*kv.0, kv.1.clone());
            let m1 = m.clone();
            let tx1 = tx.clone();
            let my_ip_and_port = self.my_ip_and_port.clone();

            thread::spawn(move || {
                let mut tcp_client = &mut tcp_client_lock.lock().unwrap();
                if my_ip_and_port != tcp_client.ip_and_port {
                    let send_res = Network::_send(&m1, &mut tcp_client);
                    if let Err(err) = send_res {
                        tcp_client.stream = None;

                        tx1.send((pub_key, Err(err))).ok();
                    } else {
                        let recv_result = Network::_recv(&mut tcp_client);

                        if recv_result.is_err() {
                            tcp_client.stream = None;
                        }

                        tx1.send((pub_key, recv_result)).ok();
                    }
                }
            });
        }
        rx
    }

    pub fn halt(&self) {
        self.alive_state.store(false, Ordering::Relaxed);
        self.server_channel.send(TCPServerCommand::Halt).unwrap();
        let mut threads = self.server_threads.lock().unwrap();
        // send dummy message to wake server thread up, in case
        let mut tcpclient = connect_to_server(self.my_ip_and_port.clone());
        let dummy = Message::Dummy;
        Network::_send(&dummy, &mut tcpclient).ok();

        for thread in threads.drain(..) {
            thread.join().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Network;
    use crate::message::{Message, TestMessage};
    use crate::signed;
    use std::collections::HashMap;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct TestState {
        state: usize,
    }

    fn modify_state(state: Arc<Mutex<TestState>>, _: Message) -> Option<Message> {
        (*state.lock().unwrap()).state += 1;
        None
    }

    fn port_adj() -> u16 {
        loop {
            let a = TcpListener::bind("127.0.0.1:0");
            if a.is_ok() {
                return a.unwrap().local_addr().unwrap().port();
            } else {
                drop(a);
            }
        }
    }

    #[test]
    fn two_network_send() {
        let test_state1 = Arc::new(Mutex::new(TestState { state: 0 }));
        let test_state2 = Arc::new(Mutex::new(TestState { state: 0 }));

        let (public1, _) = signed::gen_keys();
        let (public2, _) = signed::gen_keys();

        let ip1 = format!("127.0.0.1:{}", port_adj());
        let ip2 = format!("127.0.0.1:{}", port_adj());

        let mut signed_ip_map_1: HashMap<signed::Public, String> = HashMap::new();
        let mut signed_ip_map_2: HashMap<signed::Public, String> = HashMap::new();

        signed_ip_map_1.insert(public2, ip2.clone().to_string());
        signed_ip_map_2.insert(public1, ip1.clone().to_string());

        let network1 = Network::new(
            &ip1,
            signed_ip_map_1,
            Some((modify_state, test_state1.clone())),
        );
        let network2 = Network::new(
            &ip2,
            signed_ip_map_2,
            Some((modify_state, test_state2.clone())),
        );

        let mut a = 0;
        let mut b = 0;

        while a < 20 {
            if network1.send_to_all(&Message::Test(TestMessage { c: public1 }))[&public2].is_ok() {
                a += 1;
            }
        }

        while b < 20 {
            if network2.send_to_all(&Message::Test(TestMessage { c: public2 }))[&public1].is_ok() {
                b += 1;
            }
        }

        loop {
            if (*test_state1.lock().unwrap()).state == 20
                && (*test_state2.lock().unwrap()).state == 20
            {
                break;
            }
        }
        network1.halt();
        network2.halt();
    }
}
