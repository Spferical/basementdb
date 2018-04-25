use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use message::Message;
use signed;

const BUFFER_SIZE: usize = 8 * 1024;
const MAX_BUF_SIZE: usize = 1048576;
const READ_TIMEOUT: u64 = 30;
const WRITE_TIMEOUT: u64 = 30;
const CONNECT_TIMEOUT: u64 = 30;

#[derive(Clone)]
pub enum TCPServerCommand {
}

pub struct TCPServer {
    ip_and_port: String,
    receiver: Receiver<TCPServerCommand>,
    callback: fn(Message) -> Option<Message>,
}

pub fn read_string_from_socket(mut sock: &TcpStream) -> Option<String> {
    let mut buf: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
    let mut curr_buf: Vec<u8> = Vec::new();

    'outer: loop {
        match sock.read(&mut buf) {
            Err(err) => {
                println!("Unable to read from TCP stream: {:?}", err);
                break;
            }
            Ok(num_bytes) => {
                for i in 0..num_bytes {
                    let c = buf[i];
                    curr_buf.push(c);
                    if c == 0 {
                        break 'outer;
                    }
                }

                if num_bytes != BUFFER_SIZE {
                    break 'outer;
                }
            }
        }

        if curr_buf.len() > MAX_BUF_SIZE {
            println!("Message too long!");
            return None;
        }
    }

    match str::from_utf8(&curr_buf) {
        Err(e) => {
            println!("Invalid string read from socket: {:?}!", e);
            return None;
        }
        Ok(v) => {
            return Some(v.to_string());
        }
    };
}

pub fn start_server(
    ip_and_port: String,
    receiver: Receiver<TCPServerCommand>,
    callback: fn(Message) -> Option<Message>,
) {
    let addr: SocketAddr = ip_and_port.parse().unwrap();
    let listener = TcpListener::bind(addr).unwrap();

    let server = TCPServer {
        ip_and_port: ip_and_port,
        receiver: receiver,
        callback: callback,
    };

    for stream_result in listener.incoming() {
        match stream_result {
            Err(err) => println!("Err {:?}", err),
            Ok(stream) => {
                thread::spawn(move || handle_reader(stream, callback.clone()));
                return;
            }
        };
    }
}

fn handle_reader(mut client: TcpStream, callback: fn(Message) -> Option<Message>) {
    client
        .set_read_timeout(Some(Duration::new(READ_TIMEOUT, 0)))
        .unwrap();

    loop {
        match read_string_from_socket(&client) {
            Some(v) => {
                // Deserialize and call callback
                let potential_response = callback(Message::str_deserialize(&v));
                match potential_response {
                    Some(resp) => {
                        client
                            .set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))
                            .unwrap();
                        client.write(resp.str_serialize()).unwrap();
                        client.flush().unwrap();
                    }
                }
            }
        };
    }
}

pub struct TCPClient {
    ip_and_port: String,
    stream: TcpStream,
}

pub fn connect_to_server(ip_and_port: String) -> Option<TCPClient> {
    let sock_addr: SocketAddr = ip_and_port.parse().unwrap();
    if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::new(CONNECT_TIMEOUT, 0)) {
        return Some(TCPClient {
            ip_and_port: ip_and_port,
            stream: stream,
        });
    } else {
        println!("Failed to connect!");
        return None;
    }
}

impl TCPClient {
    pub fn send_obj(&mut self, m: String) -> bool {
        self.stream
            .set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)));
        match self.stream.write(m.as_bytes()) {
            Err(err) => {
                println!("Unable to write to TCP stream: {:?}", err);
                return false;
            }
            Ok(bytes_written) => {
                /*if bytes_written != m.as_bytes.len() {
                    println!("Hmmm... write wrote fewer bytes than expected...");
                }*/
                self.stream.flush().unwrap();
                return true;
            }
        };
    }
}

struct Network {
    peer_send_clients: Arc<Mutex<HashMap<signed::Public, Option<TCPClient>>>>,
    server_channel: Sender<TCPServerCommand>,
    my_ip_and_port: String,
}

impl Network {
    pub fn new(
        my_ip: String,
        public_key_to_ip_map: HashMap<signed::Public, String>,
        receive_callback: fn(Message) -> Option<Message>,
    ) -> Network {
        let peer_send_clients: HashMap<signed::Public, Option<TCPClient>> = public_key_to_ip_map
            .into_iter()
            .map(
                |(p, o): (signed::Public, String)| -> (signed::Public, Option<TCPClient>) {
                    (p, connect_to_server(o.to_string()))
                },
            )
            .collect();
        let (tx, rx): (Sender<TCPServerCommand>, Receiver<TCPServerCommand>) = mpsc::channel();
        let ip_copy = my_ip.clone();
        thread::spawn(move || start_server(ip_copy, rx, receive_callback));
        return Network {
            peer_send_clients: Arc::new(Mutex::new(peer_send_clients)),
            server_channel: tx,
            my_ip_and_port: my_ip.clone(),
        };
    }

    pub fn send(&mut self, m: Message, recipient: signed::Public) -> bool {
        let mut psc = self.peer_send_clients.lock().unwrap();
        let client_raw: &mut Option<TCPClient> = psc.get_mut(&recipient).unwrap();

        match client_raw {
            Some(client) => {
                let s: String = m.str_serialize();

                return client.send_obj(s);
            }
            None => {
                return false;
            }
        }
    }
}
