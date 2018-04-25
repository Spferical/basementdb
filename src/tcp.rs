use std::collections::HashMap;
use std::io;
use std::io::{Read, Write};
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

const BUFFER_SIZE: usize = 8 * 1024;
const MAX_BUF_SIZE: usize = 1048576;
const READ_TIMEOUT: u64 = 30;
const WRITE_TIMEOUT: u64 = 30;
const CONNECT_TIMEOUT: u64 = 30;

#[derive(Clone)]
pub enum TCPServerCommand {
}

pub fn read_string_from_socket(mut sock: &TcpStream) -> Result<String, io::Error> {
    let mut buf: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
    let mut curr_buf: Vec<u8> = Vec::new();

    'outer: loop {
        match sock.read(&mut buf) {
            Err(err) => {
                println!("Unable to read from TCP stream: {:?}", err);
                break;
            }
            Ok(num_bytes) => {
                if num_bytes == 0 {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Read timed out"));
                }

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
            return Err(io::Error::new(io::ErrorKind::Other, "Message too long!"));
        }
    }

    match str::from_utf8(&curr_buf) {
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

pub fn write_string_on_socket(mut sock: &TcpStream, s: &String) -> Result<(), io::Error> {
    let num_bytes: usize = sock.write(s.as_bytes())?;

    if num_bytes == 0 {
        return Err(io::Error::new(io::ErrorKind::WriteZero, "Write timed out"));
    }

    sock.flush()?;

    Ok(())
}

pub fn start_server(
    ip_and_port: String,
    receiver: Receiver<TCPServerCommand>,
    callback: fn(Message) -> Option<Message>,
) {
    let addr: SocketAddr = ip_and_port.parse().unwrap();
    let listener = TcpListener::bind(addr).unwrap();

    for stream_result in listener.incoming() {
        match stream_result {
            Err(err) => eprintln!("Err {:?}", err),
            Ok(stream) => {
                thread::spawn(move || handle_reader(stream, callback.clone()));
                return;
            }
        };
    }
}

fn handle_reader(
    client: TcpStream,
    callback: fn(Message) -> Option<Message>,
) -> Result<(), io::Error> {
    client.set_read_timeout(Some(Duration::new(READ_TIMEOUT, 0)))?;
    client.set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))?;

    loop {
        let v = read_string_from_socket(&client)?;
        let potential_response = callback(Message::str_deserialize(&v)?);
        return match potential_response {
            None => Ok(()),
            Some(resp) => write_string_on_socket(&client, &(Message::str_serialize(&resp)?)),
        };
    }
}

pub struct TCPClient {
    ip_and_port: String,
    stream: TcpStream,
}

pub fn connect_to_server(ip_and_port: String) -> Result<TCPClient, io::Error> {
    let sock_addr: SocketAddr = ip_and_port.parse().unwrap();
    if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::new(CONNECT_TIMEOUT, 0)) {
        return Ok(TCPClient {
            ip_and_port: ip_and_port,
            stream: stream,
        });
    } else {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "Failed to connect",
        ));
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
                    (p, connect_to_server(o.to_string()).ok())
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

    pub fn send(&mut self, m: Message, recipient: signed::Public) -> Result<(), io::Error> {
        let mut psc = self.peer_send_clients.lock().unwrap();
        let client_raw: &mut Option<TCPClient> = psc.get_mut(&recipient).unwrap();

        return match client_raw {
            Some(client) => {
                let s: String = Message::str_serialize(&m)?;

                &client
                    .stream
                    .set_write_timeout(Some(Duration::new(WRITE_TIMEOUT, 0)))?;

                write_string_on_socket(&client.stream, &s)
            }
            None => Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "Invalid client!",
            )),
        };
    }
}