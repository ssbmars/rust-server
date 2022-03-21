use rand::{distributions::Alphanumeric, Rng};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::collections::HashMap;
use std::net::{UdpSocket, SocketAddr};
use std::sync::mpsc;
use std::env;
use std::time::Instant;

mod packets;
mod threads;

use packets::{PacketShipper, PacketReciever, ClientPacket, ServerPacket, build_server_packet};
use threads::{create_listening_thread, create_clock_thread, ThreadMessage};

const MAX_SILENCE_DURATION: f32 = 30.0;
const MAX_PING_PONG_RATE: f32 = 5.0;

struct Session {
    key: String,
    password_protected: bool
}

struct Client {
    reciever: PacketReciever,
    shipper: PacketShipper,
    session: Option<Session>
}

struct Server {
    port: u16,
    clients: HashMap<SocketAddr, Client>,
    sessions: HashMap<String, SocketAddr>,
    valid_client_hashes: Vec<String>
}

impl Server {

    //
    // static fn
    //

    pub fn new(port: u16) -> Server {
        Server { 
            port: port, 
            clients: HashMap::new(),
            sessions: HashMap::new(),
            valid_client_hashes: Vec::new(), 
        }
    }

    fn generate_key() -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(7)
            .map(char::from)
            .collect()
    }

    pub fn poll(server: &mut Server) -> Result<(), Box<dyn std::error::Error>> {
        let ipaddr = "0.0.0.0".to_string() + ":" + &server.port.to_string();
        let socket = UdpSocket::bind(ipaddr).expect("Failed to bind host socket");

        let(tx, rx) = mpsc::channel();
        create_listening_thread(tx.clone(), socket.try_clone()?);
        create_clock_thread(tx.clone());

        println!("Server started");

        let mut time;
        let mut last_ping_pong = Instant::now();

        loop {
            match rx.recv()? {
                ThreadMessage::Tick(started) => {
                    started();

                    time = Instant::now();

                    // kick silent clients
                    let mut kick_list = Vec::new();

                    for(socket_address, client) in &mut server.clients {
                        let last_message_time = client.reciever.get_last_message_time();

                        if last_message_time.elapsed().as_secs_f32() > MAX_SILENCE_DURATION {
                            kick_list.push(*socket_address);
                            continue;
                        }

                        // start ping-pong
                        if last_ping_pong.elapsed().as_secs_f32() >= MAX_PING_PONG_RATE {
                            client.shipper.send(&socket, &ServerPacket::Ping);
                            last_ping_pong = time;
                        }

                       client.shipper.resend_unacknowledged_packets(&socket);
                    }

                    for socket_address in kick_list {
                        let buf = build_server_packet(&ServerPacket::Close);
                        let _ = socket.send_to(&buf, socket_address);

                        println!("Dropping host {} due to silence", socket_address.to_string());
                        server.drop_client(&socket_address);
                    }
                }
                ThreadMessage::ClientPacket {
                    socket_address,
                    id,
                    packet
                } => {
                    if server.has_client(&socket_address) {
                        let reciever = &mut server.clients.get_mut(&socket_address).unwrap().reciever;
                        
                        if let Some(data) = reciever.sort_packets(&socket, id, packet) {
                            server.handle_packet(&socket, socket_address, id, data)
                        }
                    } else {
                        // new connection
                        let mut client = Client { 
                            reciever: PacketReciever::new(socket_address),
                            shipper: PacketShipper::new(socket_address),
                            session: None
                        };
    
                        let reciever = &mut client.reciever;

                        if let Some(data) = reciever.sort_packets(&socket, id, packet) {
                            server.clients.insert(socket_address, client);

                            println!("Some data packet ID is {} from {}", id, socket_address);
                            server.handle_packet(&socket, socket_address, id, data)
                        }
                    }
                }
            }
        }
    }

    fn handle_packet(&mut self, socket: &UdpSocket, socket_address: SocketAddr, id: u32, packet: ClientPacket) {
        if self.has_client(&socket_address) {
            match packet {
                ClientPacket::Pong => {},
                ClientPacket::Ack { id } => {
                    self.clients.get_mut(&socket_address).unwrap().shipper.acknowledge(id);
                },
                ClientPacket::Create { client_hash, password_protected } => {
                    if !self.valid_client_hash(&client_hash) {
                        println!("client hash {} is not valid", client_hash);
                        return;
                    }

                    if let Some(key) = self.create_session(&socket_address, password_protected) {
                        let reply = ServerPacket::Create{ session_key: &key };
                        self.clients.get_mut(&socket_address).unwrap().shipper.send(socket, &reply);
                    } else {
                        let reply = ServerPacket::Error{ id, message: &"Session failed to create" };
                        self.clients.get_mut(&socket_address).unwrap().shipper.send(socket, &reply);
                    }
                },
                ClientPacket::Join { client_hash, session_key } => {
                    if !self.valid_client_hash(&client_hash) {
                        return;
                    }

                    if session_key.is_empty() {
                        if let Some(client_addr) = self.get_socket_addr_from_open_session(&socket_address) {
                            // send to requester
                            self.clients
                            .get_mut(&socket_address)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: Some(&client_addr), success: true });
                            
                            // send to session host
                            self.clients
                            .get_mut(&client_addr)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: Some(&socket_address), success: true });

                            // Drop any sessions related to these two clients
                            self.drop_client_session(&client_addr);
                            self.drop_client_session(&socket_address);
                        } else {
                            self.clients
                            .get_mut(&socket_address)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: None, success: false });
                        }
                    } else {
                        if let Some(client_addr) = self.get_socket_addr_from_session(&session_key, &socket_address) {
                            // send to requester
                            self.clients
                            .get_mut(&socket_address)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: Some(&client_addr), success: true });
                            
                            // send to session host
                            self.clients
                            .get_mut(&client_addr)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: Some(&socket_address), success: true });

                            // Drop any sessions related to these two clients
                            self.drop_client_session(&client_addr);
                            self.drop_client_session(&socket_address);
                        } else {
                            self.clients
                            .get_mut(&socket_address)
                            .unwrap()
                            .shipper
                            .send(socket, &ServerPacket::Join{ client_addr: None, success: false });
                        }
                    }
                },
                ClientPacket::Close => {
                    self.drop_client_session(&socket_address);
                }
            }
        }
    }

    //
    // non mut fn
    //

    fn has_key(&self, key: &str) -> bool {
        self.sessions.contains_key(key)
    }

    fn has_client(&self, socket_address: &SocketAddr) -> bool {
        self.clients.contains_key(socket_address)
    }

    fn has_session(&self, socket_address: &SocketAddr) -> bool {
        let result = self
        .sessions
        .iter()
        .find_map(|(key, &val)| if val == *socket_address { Some(key) } else { None });

        if let Some(_) = result {
            return true;
        }

        false
    }

    fn valid_client_hash(&self, hash: &str) -> bool {
        self.valid_client_hashes.iter().position(|h: &String| *h == *hash) != None
    }

    fn get_socket_addr_from_session(&self, key: &str, exclude_socket: &SocketAddr) -> Option<SocketAddr> {
        if let Some(socket) = self.sessions.get(key) {
            if exclude_socket != socket {
                return Some(socket.clone())
            }
        }

        None
    }

    fn get_socket_addr_from_open_session(&self, exclude_socket: &SocketAddr) -> Option<SocketAddr> {
        self.sessions
            .values()
            .find(|client_socket| {
                self.clients.get(&client_socket).unwrap().session.as_ref().unwrap().password_protected == false 
                && *client_socket != exclude_socket
            })
            .cloned()
    }

    //
    // mut fn
    //

    pub fn support_client_hashes(&mut self, hashes: Vec<String>) {
        self.valid_client_hashes = hashes;
    }

    fn create_session(&mut self, socket_address: &SocketAddr, password_protected: bool) -> Option<String> {
        let mut result = None;

        if !self.has_session(&socket_address) {
            loop {
                let new_key = Server::generate_key();

                if !self.has_key(&new_key) {
                    let session = Session { key: new_key.clone(), password_protected: password_protected };

                    let client = &mut self.clients.get_mut(socket_address).unwrap();
                    client.session = Some(session);
                    
                    self.sessions.insert(new_key.clone(), socket_address.clone());

                    println!("Session created for client {}:{} with key {} (password_protected: {})", 
                        socket_address.ip(), 
                        socket_address.port(), 
                        new_key, 
                        password_protected
                    );

                    result = Some(new_key);
                    break;
                }
            }
        } else {
            println!("Session for {}:{} cannot be created because it already exists", 
                socket_address.ip(), 
                socket_address.port()
            );
        }

        result
    }

    // Drop the client session only (when a match is made)
    fn drop_client_session(&mut self, socket_address: &SocketAddr) -> bool {
        if let Some(client) = self.clients.get(socket_address) {
            if let Some(session) = &client.session {
                self.sessions.remove(&session.key);
            }

            return true;
        }

        false
    }

    // Drop the client entirely including associated resources
    fn drop_client(&mut self, socket_address: &SocketAddr) -> bool {
        if let Some(client) = self.clients.remove(socket_address) {
            if let Some(session) = client.session {
                self.sessions.remove(&session.key);
            }

            return true;
        }

        false
    }
}

//
// util fn
//

fn file_read_lines(path: &str) -> Vec<String> {
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut result = Vec::new();

    // Read the file line by line using the lines() iterator from std::io::BufRead.
    for (_, line) in reader.lines().enumerate() {
        let line = line.unwrap(); // Ignore errors
        result.push(line);
    }

    result
}

#[allow(dead_code)]
fn print_key(key: &Option<String>) {
    match key {
        Some(x) => println!("Key was {}", x),
        None => println!("Empty key")
    }
}

#[allow(dead_code)]
fn test_hash(server: &Server, hash: &String) {
    println!("Hash {} is supported by server: {}", hash, server.valid_client_hash(hash));
}

//
// entry
// 

fn main() {
    let port: u16;

    match env::args().nth(1) {
        Some(arg) => {
            match arg.parse::<u16>() {
                Ok(x) => {
                    port = x;
                },
                Err(_) => {
                    println!("Aborting! Port number must be an integer sequence only!");
                    return;
                }
            }
        },
        None => {
            println!("Supply a port number!");
            return;
        }
    }

    let mut server = Server::new(port);

    server.support_client_hashes(file_read_lines("./hashes.txt"));

    match Server::poll(&mut server) {
        Ok(_) => {
            println!("Server closed.");
        },
        Err(e) =>{
            println!("Server encountered an error: {}", e.to_string());
        }
    }
}
