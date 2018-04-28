use bufstream::BufStream;
use std::collections::HashMap;
use std::io;
use std::io::BufRead;
use std::io::Write;
use std::marker::Send;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use message::Message;
use signed;
use str_serialize::StrSerialize;

const MAX_BUF_SIZE: usize = 1048576;
const READ_TIMEOUT: u64 = 30;
const WRITE_TIMEOUT: u64 = 30;
const CONNECT_TIMEOUT: u64 = 30;

#[derive(Clone)]
pub enum TCPServerCommand {
    Halt,
}

pub fn read_string_from_socket(sock: &mut BufStream<TcpStream>) -> Result<String, io::Error> {
    let mut buf: Vec<u8> = Vec::new();

    match sock.read_until(0, &mut buf) {
        Err(err) => {
            println!("Unable to read from TCP stream: {:?}", err);
        }
        Ok(num_bytes) => {
            if num_bytes == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Read timed out"));
            } else {
                // remove the null byte
                let l = buf.len();
                buf.remove(l - 1);
            }
        }
    }

    match str::from_utf8(&buf) {
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Read invalid string from socket {:?}!", e),
            ));
        }
        Ok(v) => {
            return Ok(v.to_string());
        }
    };
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

type ServerCallback<T> = (fn(T, Message, Network) -> Option<Message>, T);

fn invoke<T: Clone>(
    callback: ServerCallback<T>,
    message: Message,
    net: Network,
) -> Option<Message> {
    return callback.0(callback.1, message, net);
}

pub fn start_server<'a, T: 'static + Send + Clone>(
    net: Network,
    receiver: Receiver<TCPServerCommand>,
    callback: ServerCallback<T>,
) {
    let ip_and_port = net.my_ip_and_port.clone();
    let addr: SocketAddr = ip_and_port.parse().unwrap();
    let listener = TcpListener::bind(addr).unwrap();
    let alive = Arc::new(Mutex::new(true));

    for stream_result in listener.incoming() {
        match stream_result {
            Err(err) => eprintln!("Err {:?}", err),
            Ok(stream) => {
                let net1 = net.clone();
                thread::spawn(move || handle_reader(net1, stream, callback, alive));
                return;
            }
        };

        match receiver.try_recv() {
            Err(_) => continue,
            Ok(cmd) => match cmd {
                TCPServerCommand::Halt => {
                    *(alive.lock().unwrap()) = false;
                    return;
                }
            },
        }
    }
}

fn handle_reader<T: Clone>(
    net: Network,
    client: TcpStream,
    callback: ServerCallback<T>,
    alive: Arc<Mutex<bool>>,
) -> Result<(), io::Error> {
    client.set_read_timeout(Some(Duration::new(READ_TIMEOUT, 0)))?;
    client.set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))?;

    let mut bufclient = BufStream::with_capacities(MAX_BUF_SIZE, MAX_BUF_SIZE, client);

    loop {
        let v = read_string_from_socket(&mut bufclient)?;

        // Lets check if we're still alive...
        {
            if !*(alive.lock().unwrap()) {
                return Ok(());
            }
        }

        let potential_response =
            invoke(callback.clone(), Message::str_deserialize(&v)?, net.clone());
        match potential_response {
            None => {}
            Some(resp) => write_string_on_socket(&mut bufclient, Message::str_serialize(&resp)?)?,
        };
    }
}

#[derive(Debug)]
pub struct TCPClient {
    ip_and_port: String,
    stream: Result<TcpStream, io::Error>,
}

pub fn connect_to_server(ip_and_port: String) -> TCPClient {
    let sock_addr: SocketAddr = ip_and_port.parse().unwrap();
    let stream = TcpStream::connect_timeout(&sock_addr, Duration::new(CONNECT_TIMEOUT, 0));
    return TCPClient {
        ip_and_port: ip_and_port,
        stream: stream,
    };
}

fn try_connecting_to_everyone(
    h: HashMap<signed::Public, String>,
) -> HashMap<signed::Public, TCPClient> {
    h.into_iter()
        .map(
            |(p, o): (signed::Public, String)| -> (signed::Public, TCPClient) {
                let r1 = connect_to_server(o.to_string());
                match &r1.stream {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Tried to connect, but failed due to: {:?}", e);
                    }
                };
                (p, r1)
            },
        )
        .collect()
}

fn retry_dead_connections(
    p: Arc<Mutex<HashMap<signed::Public, TCPClient>>>,
    alive: Arc<Mutex<bool>>,
) {
    loop {
        {
            if !(*alive.lock().unwrap()) {
                return;
            }
        }

        thread::sleep(Duration::new(1, 0));

        let retries: HashMap<signed::Public, String>;

        {
            let conns = &*p.lock().unwrap();
            retries = conns
                .into_iter()
                .filter(|(_, v)| v.stream.is_err())
                .map(|(p, v)| (p.clone(), v.ip_and_port.clone()))
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
    peer_send_clients: Arc<Mutex<HashMap<signed::Public, TCPClient>>>,
    server_channel: Sender<TCPServerCommand>,
    alive_state: Arc<Mutex<bool>>,
    my_ip_and_port: String,
}

impl Network {
    pub fn new<'a, T: 'static + Send + Clone>(
        my_ip: String,
        public_key_to_ip_map: HashMap<signed::Public, String>,
        receive_callback: Option<ServerCallback<T>>,
    ) -> Network {
        let peer_send_clients: HashMap<signed::Public, TCPClient> =
            try_connecting_to_everyone(public_key_to_ip_map);
        let (tx, rx): (Sender<TCPServerCommand>, Receiver<TCPServerCommand>) = mpsc::channel();
        let psc = Arc::new(Mutex::new(peer_send_clients));
        let psc1 = psc.clone();
        let alive = Arc::new(Mutex::new(true));
        let alive1 = alive.clone();
        let net = Network {
            peer_send_clients: psc,
            server_channel: tx,
            alive_state: alive,
            my_ip_and_port: my_ip.clone(),
        };
        let net1 = net.clone();

        if receive_callback.is_some() {
            thread::spawn(move || start_server(net1, rx, receive_callback.unwrap()));
        }
        thread::spawn(move || retry_dead_connections(psc1, alive1));

        return net;
    }

    pub fn send(&mut self, m: Message, recipient: signed::Public) -> Result<(), io::Error> {
        let mut psc = self.peer_send_clients.lock().unwrap();
        let client_raw: &mut TCPClient = psc.get_mut(&recipient).unwrap();

        return match &client_raw.stream {
            Ok(stream) => {
                let s: String = Message::str_serialize(&m)?;

                stream.set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))?;

                write_string_on_socket(stream, s)
            }
            Err(e) => Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("Invalid client... it failed due to: {:?}", e),
            )),
        };
    }

    pub fn send_to_all(&mut self, m: Message) -> HashMap<signed::Public, Result<(), io::Error>> {
        let psc = self.peer_send_clients.lock().unwrap();

        return (psc)
            .iter()
            .filter(|(_, v)| self.my_ip_and_port != v.ip_and_port)
            .map(|(&p, _)| {
                let new_self: &mut Network = &mut (self.clone());
                (p, new_self.send(m.clone(), p))
            })
            .collect();
    }

    pub fn halt(&self) {
        *self.alive_state.lock().unwrap() = false;
        self.server_channel.send(TCPServerCommand::Halt).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::Network;
    use message::{Message, TestMessage, UnsignedMessage};
    use signed;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct TestState {
        state: usize,
    }

    fn modify_state(state: Arc<Mutex<TestState>>, _: Message, _: Network) -> Option<Message> {
        (*state.lock().unwrap()).state += 1;
        None
    }

    #[test]
    fn two_network_send() {
        let test_state1 = Arc::new(Mutex::new(TestState { state: 0 }));
        let test_state2 = Arc::new(Mutex::new(TestState { state: 0 }));

        let (public1, private1) = signed::gen_keys();
        let (public2, private2) = signed::gen_keys();

        let ip1 = "127.0.0.1:54321";
        let ip2 = "127.0.0.1:54320";

        let mut signed_ip_map_1: HashMap<signed::Public, String> = HashMap::new();
        let mut signed_ip_map_2: HashMap<signed::Public, String> = HashMap::new();

        signed_ip_map_1.insert(public2, ip2.clone().to_string());
        signed_ip_map_2.insert(public1, ip1.clone().to_string());

        let mut network1 = Network::new(
            ip1.to_string(),
            signed_ip_map_1,
            Some((modify_state, test_state1.clone())),
        );
        let mut network2 = Network::new(
            ip2.to_string(),
            signed_ip_map_2,
            Some((modify_state, test_state2.clone())),
        );

        let mut a = 0;
        let mut b = 0;

        while a < 20 {
            if network1
                .send(
                    Message::Unsigned(UnsignedMessage::Test(TestMessage { c: public1 })),
                    public2,
                )
                .is_ok()
            {
                a += 1;
            }
        }

        while b < 20 {
            if network2
                .send(
                    Message::Unsigned(UnsignedMessage::Test(TestMessage { c: public2 })),
                    public1,
                )
                .is_ok()
            {
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
    }
}
