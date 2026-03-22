use std::net::IpAddr;
use anyhow::Result;
use bytes::{BufMut, BytesMut};
use tokio::net::UdpSocket;
use clap::Parser;

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
    let mut buf = [0u8; 1024];

    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        if len > 0 {
            let command = buf[0];
            if command == 0 {
                socket.send_to(&[0, 1], addr).await?;
            } else if command == 1 {
                if let IpAddr::V4(v4) = addr.ip() {
                    let mut r = BytesMut::with_capacity(5);
                    r.put_u8(1);
                    r.put_slice(&v4.octets());
                    socket.send_to(&r, addr).await?;
                }
            };
        }
    }
}
