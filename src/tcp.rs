use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::thread;
use std::time::Duration;

use message::Message;

const MAX_BUF_SIZE: usize = 1048576;
const READ_TIMEOUT: u64 = 30;
const WRITE_TIMEOUT: u64 = 30;
const CONNECT_TIMEOUT: u64 = 30;

#[derive(Clone)]
pub struct TCPServer {
    ip_and_port: String,
    callback: fn(Message),
}

pub fn read_string_from_socket(mut sock: &TcpStream) -> Option<String> {
    let mut buf: [u8; 256] = [0; 256];
    let mut curr_buf: Vec<u8> = Vec::new();

    'outer: loop {
        match sock.read(&mut buf) {
            Err(err) => println!("Unable to read from TCP stream: {:?}", err),
            Ok(num_bytes) => {
                for i in 0..num_bytes {
                    let c = buf[i];
                    curr_buf.push(c);
                    if c == 0 {
                        break 'outer;
                    }
                }

                if num_bytes != 256 {
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

pub fn start_server(ip_and_port: String, callback: fn(Message)) {
    let addr: SocketAddr = ip_and_port.parse().unwrap();
    let listener = TcpListener::bind(addr).unwrap();

    let server = TCPServer {
        ip_and_port: ip_and_port,
        callback: callback,
    };

    for stream_result in listener.incoming() {
        let new_self = server.clone();
        match stream_result {
            Err(err) => println!("Err {:?}", err),
            Ok(stream) => {
                thread::spawn(move || new_self.handle_client(stream));
                return;
            }
        };
    }
}

impl TCPServer {
    fn handle_client(&self, mut client: TcpStream) {
        client.set_read_timeout(Some(Duration::new(READ_TIMEOUT, 0)));

        loop {
            let s: String;
            match read_string_from_socket(&client) {
                None => {
                    return;
                }
                Some(v) => s = v,
            };

            // Deserialize and call callback
        }
    }
}

pub struct TCPClient {
    ip_and_port: String,
    stream: TcpStream,
    callback: fn(Message),
}

pub fn connect_to_server(ip_and_port: String, callback: fn(Message)) -> Option<TCPClient> {
    let sock_addr: SocketAddr = ip_and_port.parse().unwrap();
    if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, Duration::new(CONNECT_TIMEOUT, 0)) {
        return Some(TCPClient {
            ip_and_port: ip_and_port,
            stream: stream,
            callback: callback,
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
                return true;
            }
        };
    }
}
