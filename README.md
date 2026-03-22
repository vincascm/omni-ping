# omni-ping

A lightweight, real-time UDP ping-pong tool written in Rust, featuring a web-based dashboard with geographic insights and drill-down statistics.

## Features

- **Real-time Monitoring**: Track latency (Min/Avg/Max/Std Dev) and packet loss in real-time.
- **Geographic Insights**: Automatic IP-based geolocation for both local client and remote server.
- **Physical Limit Analysis**: Calculates the theoretical fiber-optic latency limit based on great-circle distance.
- **Interactive Dashboard**: Built-in web server with Chart.js for data visualization.
- **Drill-down Analytics**: Click on any aggregated data point (e.g., an hour) to zoom in and see detailed second-by-second or minute-by-minute logs for that period.
- **Dual Binary Architecture**: Separate client and server binaries for minimal footprint on the server side.

## Installation

Ensure you have [Rust](https://www.rust-lang.org/) installed.

```bash
git clone https://github.com/your-username/omni-ping.git
cd omni-ping
cargo build --release
```

The binaries will be available in `target/release/client` and `target/release/server`.

## Usage

### 1. Run the Server

On your remote machine, start the UDP echo server:

```bash
./target/release/server --bind 0.0.0.0:51800
```

### 2. Run the Client

On your local machine, start the pinger and the stats dashboard:

```bash
./target/release/client --server <SERVER_IP>:51800 --listen 127.0.0.1:3000
```

### Options

**Server:**
- `-b, --bind <ADDR>`: Address to bind the UDP socket (default: `0.0.0.0:51800`).

**Client:**
- `-s, --server <ADDR>`: Target server address (IP:Port or Host:Port).
- `-l, --listen <ADDR>`: HTTP address for the dashboard (default: `127.0.0.1:3000`).
- `-i, --interval <MS>`: Ping interval in milliseconds (default: `1000`).
- `-t, --timeout <SEC>`: Ping timeout in seconds (default: `2`).
- `-m, --max-entries <N>`: Maximum history entries to keep in memory (default: `1,000,000`).

## Web Dashboard

Once the client is running, open `http://127.0.0.1:3000` in your browser.

- **Main View**: Shows historical trends aggregated by Second, Minute, Hour, or Day depending on total duration.
- **Drill-down**: Click on a bar/point in the chart to "zoom in" to that specific time range.
- **Reset**: Click the "Reset Zoom" button next to the title to return to the global view.

## License

MIT
