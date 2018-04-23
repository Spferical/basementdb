use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::thread;
use std::time::Duration;

use message::Message;

const MAX_BUF_SIZE: usize = 1048576;
const READ_TIMEOUT: Duration = Duration::new(30, 0);
const WRITE_TIMEOUT: Duration = Duration::new(30, 0);
const CONNECT_TIMEOUT: Duration = Duration::new(30, 0);

#[derive(Clone)]
struct TCPServer {
    ip_and_port: String,
}

impl TCPServer {
    fn start_server(self) {
        let addr: SocketAddr = self.ip_and_port.parse().unwrap();
        let listener = TcpListener::bind(addr).unwrap();

        for stream_result in listener.incoming() {
            let new_self = self.clone();
            match stream_result {
                Err(err) => println!("Err {:?}", err),
                Ok(stream) => {
                    thread::spawn(move || new_self.handle_client(stream));
                    return;
                }
            };
        }
    }

    fn handle_client(&self, mut client: TcpStream) {
        client.set_read_timeout(Some(READ_TIMEOUT));
        let mut buf: [u8; 256] = [0; 256];
        let mut curr_buf: Vec<u8> = Vec::new();
        loop {
            match client.read(&mut buf) {
                Err(err) => println!("Unable to read from TCP stream: {:?}", err),
                Ok(num_bytes) => {
                    for i in 0..num_bytes {
                        let c = buf[i];
                        curr_buf.push(c);
                        if c == 0 {
                            break;
                        }
                    }

                    if num_bytes != 256 {
                        break;
                    }
                }
            }

            if curr_buf.len() > MAX_BUF_SIZE {
                println!("Message too long!");
                return;
            }
        }

        let s: String;
        match str::from_utf8(&curr_buf) {
            Err(e) => {
                println!("Invalid string read from socket: {:?}!", e);
                return;
            }
            Ok(v) => {
                s = v.to_string();
            }
        };
    }
}

struct TCPClient {
    ip_and_port: String,
    stream: TcpStream,
    callback: Fn(Message),
}

impl TCPClient {
    fn connect_to_server(&self) -> bool {
        let sock_addr: SocketAddr = self.ip_and_port.parse().unwrap();
        if let Ok(stream) = TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT) {
            self.stream = stream;
            return true;
        } else {
            println!("Failed to connect!");
            return false;
        }
    }

    fn send_obj(&self, m: String) -> bool {
        self.stream.set_write_timeout(Some(WRITE_TIMEOUT));
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
