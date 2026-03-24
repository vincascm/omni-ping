use anyhow::Result;
use bytes::{BufMut, Bytes, BytesMut};
use clap::Parser;
use std::net::{IpAddr, SocketAddr};
use tokio::net::UdpSocket;

#[derive(Parser)]
#[command(name = "omni-ping-server")]
#[command(about = "UDP Ping-Pong Server")]
struct Cli {
    #[arg(short, long, default_value = "0.0.0.0:51800")]
    bind: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = UdpSocket::bind(&cli.bind).await?;
    println!("Server listening on {}", cli.bind);
    let mut buf = [0u8; 8];

    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        if len > 0 {
            let mut r = BytesMut::new();
            if buf[0] == 1 {
                r.put_u8(1);
                r.put_slice(&socket_addr_to_bytes(addr));
            } else {
                r.put_u8(0);
                r.put_u8(1);
            };
            socket.send_to(&r, addr).await?;
        }
    }
}

fn socket_addr_to_bytes(socket_addr: SocketAddr) -> Bytes {
    let mut r = BytesMut::new();
    match socket_addr.ip() {
        IpAddr::V4(ip) => {
            r.put_u8(0);
            r.put_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            r.put_u8(1);
            r.put_slice(&ip.octets());
        }
    }
    r.put_u16(socket_addr.port());
    r.freeze()
}
