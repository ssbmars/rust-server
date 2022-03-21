use crate::packets::parse_client_packet;
use crate::threads::ThreadMessage;
use std::net::UdpSocket;
use std::sync::mpsc;

pub fn create_listening_thread(tx: mpsc::Sender<ThreadMessage>, socket: UdpSocket) {
    let async_socket = async_std::net::UdpSocket::from(socket);
    async_std::task::spawn(listen_loop(tx, async_socket));
}

async fn listen_loop(tx: mpsc::Sender<ThreadMessage>, async_socket: async_std::net::UdpSocket) {
    loop {
        let mut buf = vec![0; 1024];

        let wrapped_packet = async_socket.recv_from(&mut buf).await;

        if wrapped_packet.is_err() {
            // don't crash if there's an error...
            continue;
        }

        let (number_of_bytes, src_addr) = wrapped_packet.unwrap();
        let data = &buf[..number_of_bytes];

        if let Some((id, packet)) = parse_client_packet(data) {
            tx.send(ThreadMessage::ClientPacket {
                socket_address: src_addr,
                id,
                packet
            })
            .unwrap();
        } else {
            println!("Receive unknown packet from {}", src_addr);
            println!("{:?}", data);
        }
    }
}