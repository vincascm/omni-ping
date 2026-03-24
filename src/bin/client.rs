use clap::Parser;
use std::{
    cmp::min,
    collections::{BTreeMap, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Instant,
};

use anyhow::{Result, anyhow};
use axum::{
    Router,
    extract::{Query, State},
    response::{Html, Json},
    routing::get,
};
use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};

use tokio::{
    net::{TcpListener, UdpSocket, lookup_host},
    signal,
    sync::RwLock,
    time::{Duration, interval, sleep},
};

#[derive(Parser)]
#[command(name = "omni-ping-client")]
#[command(about = "UDP Ping-Pong Client")]
struct Cli {
    /// Server address to ping
    #[arg(short, long)]
    server: String,
    /// Timeout for each ping in seconds
    #[arg(short, long, default_value = "2")]
    timeout: u64,
    /// Interval between pings in milliseconds
    #[arg(short, long, default_value = "1000")]
    interval: u64,
    /// Maximum entries to keep in memory (default 1M ~ 100MB)
    #[arg(short, long, default_value = "1000000")]
    max_entries: usize,
    /// Address to bind for stats server
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    listen: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct GeoInfo {
    query: String,
    status: String,
    lat: Option<f64>,
    lon: Option<f64>,
    city: Option<String>,
    country: Option<String>,
    #[serde(rename = "as")]
    isp_as: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct ZoomParams {
    pub start: Option<i64>,
    pub end: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
struct ChartPoint {
    label: String,
    avg_latency: Option<f64>,
    timeout_percent: f64,
    start_ts: i64,
    end_ts: i64,
}

#[derive(Serialize, Clone, Debug)]
struct FinalStats {
    target: String,
    server_geo: Option<GeoInfo>,
    client_geo: Option<GeoInfo>,
    distance_km: Option<f64>,
    fiber_limit_ms: Option<f64>,
    start_time_str: String,
    end_time_str: String,
    total_transmitted: usize,
    total_received: usize,
    packet_loss_percent: f64,
    min_latency: String,
    avg_latency: String,
    max_latency: String,
    std_dev: String,
    total_duration: String,
    points: Vec<ChartPoint>,
    interval_name: String,
    is_zoomed: bool,
}

struct Stats {
    target: String,
    server_geo: Option<GeoInfo>,
    client_geo: Option<GeoInfo>,
    entries: VecDeque<(DateTime<Local>, Option<Duration>)>,
    start_time: Instant,
    max_entries: usize,
    total_sent: usize,
}

impl Stats {
    fn new(target: &str, max_entries: usize) -> Self {
        Self {
            target: target.to_string(),
            server_geo: None,
            client_geo: None,
            entries: VecDeque::with_capacity(min(max_entries, 10000)),
            start_time: Instant::now(),
            max_entries,
            total_sent: 0,
        }
    }

    fn record(&mut self, timestamp: DateTime<Local>, latency: Option<Duration>) {
        self.total_sent += 1;
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back((timestamp, latency));
    }

    fn calculate_distance(
        &self,
        server_geo: &Option<GeoInfo>,
        client_geo: &Option<GeoInfo>,
    ) -> Option<f64> {
        let s = server_geo.as_ref()?;
        let c = client_geo.as_ref()?;
        let s_lat = s.lat?;
        let s_lon = s.lon?;
        let c_lat = c.lat?;
        let c_lon = c.lon?;
        let r = 6371.0;
        let d_lat = (c_lat - s_lat).to_radians();
        let d_lon = (c_lon - s_lon).to_radians();
        let a = (d_lat / 2.0).sin().powi(2)
            + s_lat.to_radians().cos() * c_lat.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        Some(r * c)
    }

    fn to_final_stats(&self, query: ZoomParams) -> FinalStats {
        let is_zoomed = query.start.is_some() || query.end.is_some();
        let entries_filtered: Vec<_> = self
            .entries
            .iter()
            .filter(|(ts, _)| {
                if let Some(s) = query.start
                    && ts.timestamp() < s
                {
                    return false;
                }
                if let Some(e) = query.end
                    && ts.timestamp() > e
                {
                    return false;
                }
                true
            })
            .collect();

        let current_entries = entries_filtered.len();
        if current_entries == 0 {
            return self.empty_stats();
        }
        let first_ts = entries_filtered.first().unwrap().0;
        let last_ts = entries_filtered.last().unwrap().0;
        let span = last_ts.signed_duration_since(first_ts);

        let (interval_name, group_key_fn): (&str, fn(DateTime<Local>) -> String) =
            if span.num_minutes() < 10 {
                ("Raw (Sec)", |ts| ts.format("%H:%M:%S").to_string())
            } else if span.num_hours() < 6 {
                ("Minute", |ts| ts.format("%H:%M").to_string())
            } else if span.num_days() < 3 {
                ("Hour", |ts| ts.format("%m-%d %H:00").to_string())
            } else if span.num_weeks() < 4 {
                ("Day", |ts| ts.format("%Y-%m-%d").to_string())
            } else {
                ("Week", |ts| {
                    format!("{}-W{}", ts.year(), ts.iso_week().week())
                })
            };

        let mut groups: BTreeMap<String, (Vec<f64>, usize, i64, i64)> = BTreeMap::new();
        let mut total_received_all = 0;
        let mut latencies_all = Vec::new();

        for (ts, lat) in &entries_filtered {
            let key = group_key_fn(*ts);
            let entry =
                groups
                    .entry(key)
                    .or_insert((Vec::new(), 0, ts.timestamp(), ts.timestamp()));

            if ts.timestamp() < entry.2 {
                entry.2 = ts.timestamp();
            }
            if ts.timestamp() > entry.3 {
                entry.3 = ts.timestamp();
            }

            match lat {
                Some(d) => {
                    let ms = d.as_secs_f64() * 1000.0;
                    entry.0.push(ms);
                    latencies_all.push(ms);
                    total_received_all += 1;
                }
                None => {
                    entry.1 += 1;
                }
            }
        }

        let points: Vec<ChartPoint> = groups
            .into_iter()
            .map(|(label, (lats, timeouts, min_ts, max_ts))| {
                let total = lats.len() + timeouts;
                ChartPoint {
                    label,
                    avg_latency: if lats.is_empty() {
                        None
                    } else {
                        Some(lats.iter().sum::<f64>() / lats.len() as f64)
                    },
                    timeout_percent: (timeouts as f64 / total as f64) * 100.0,
                    start_ts: min_ts,
                    end_ts: max_ts,
                }
            })
            .collect();

        let mut min_ms = 0.0;
        let mut max_ms = 0.0;
        let mut avg_ms = 0.0;
        let mut std_dev_ms = 0.0;

        if !latencies_all.is_empty() {
            min_ms = *latencies_all
                .iter()
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();
            max_ms = *latencies_all
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();
            let sum: f64 = latencies_all.iter().sum();
            let count = latencies_all.len() as f64;
            avg_ms = sum / count;
            let variance = latencies_all
                .iter()
                .map(|ms| (ms - avg_ms).powi(2))
                .sum::<f64>()
                / count;
            std_dev_ms = variance.sqrt();
        }

        let distance_km = self.calculate_distance(&self.server_geo, &self.client_geo);
        let fiber_limit_ms = distance_km.map(|d| (d * 2.0) / (299.792458 / 1.47));

        FinalStats {
            target: self.target.clone(),
            server_geo: self.server_geo.clone(),
            client_geo: self.client_geo.clone(),
            distance_km,
            fiber_limit_ms,
            start_time_str: first_ts.format("%Y-%m-%d %H:%M:%S").to_string(),
            end_time_str: last_ts.format("%Y-%m-%d %H:%M:%S").to_string(),
            total_transmitted: self.total_sent,
            total_received: total_received_all,
            packet_loss_percent: if current_entries > 0 {
                (current_entries - total_received_all) as f64 / current_entries as f64 * 100.0
            } else {
                0.0
            },
            min_latency: format!("{:.2?}ms", min_ms),
            avg_latency: format!("{:.2?}ms", avg_ms),
            max_latency: format!("{:.2?}ms", max_ms),
            std_dev: format!("{:.2?}ms", std_dev_ms),
            total_duration: format!("{:?}", self.start_time.elapsed()),
            points,
            interval_name: interval_name.to_string(),
            is_zoomed,
        }
    }

    fn empty_stats(&self) -> FinalStats {
        FinalStats {
            target: self.target.clone(),
            server_geo: None,
            client_geo: None,
            distance_km: None,
            fiber_limit_ms: None,
            start_time_str: "N/A".into(),
            end_time_str: "N/A".into(),
            total_transmitted: 0,
            total_received: 0,
            packet_loss_percent: 0.0,
            min_latency: "0ms".into(),
            avg_latency: "0ms".into(),
            max_latency: "0ms".into(),
            std_dev: "0ms".into(),
            total_duration: "0s".into(),
            points: Vec::new(),
            interval_name: "None".into(),
            is_zoomed: false,
        }
    }
}

async fn fetch_geo(ip_or_host: IpAddr) -> Result<GeoInfo> {
    let url = format!("http://ip-api.com/json/{ip_or_host}");
    Ok(reqwest::get(&url).await?.json().await?)
}

fn parse_socket_addr(data: &[u8]) -> Result<SocketAddr> {
    match data[0] {
        0 if data.len() == 7 => {
            let octets: [u8; 4] = data[1..5].try_into()?;
            let ip = Ipv4Addr::from_octets(octets);
            let octets: [u8; 2] = data[5..7].try_into()?;
            let port = u16::from_be_bytes(octets);
            Ok(SocketAddr::new(IpAddr::V4(ip), port))
        }
        1 if data.len() == 19 => {
            let octets: [u8; 16] = data[1..17].try_into()?;
            let ip = Ipv6Addr::from_octets(octets);
            let octets: [u8; 2] = data[17..19].try_into()?;
            let port = u16::from_be_bytes(octets);
            Ok(SocketAddr::new(IpAddr::V6(ip), port))
        }
        _ => Err(anyhow!("invalid socket addr bytes")),
    }
}

async fn show_index() -> Html<&'static str> {
    Html(include_str!("../../stats.html"))
}

async fn get_stats_api(
    State(state): State<Arc<RwLock<Stats>>>,
    Query(query): Query<ZoomParams>,
) -> Json<FinalStats> {
    Json(state.read().await.to_final_stats(query))
}

impl Cli {
    async fn run(&self) -> Result<()> {
        let server_addr = lookup_host(&self.server)
            .await?
            .next()
            .ok_or_else(|| anyhow!("Could not resolve: {}", self.server))?;
        let stats = Arc::new(RwLock::new(Stats::new(&self.server, self.max_entries)));
        let stats_for_geo = stats.clone();
        tokio::spawn(async move {
            match fetch_geo(server_addr.ip()).await {
                Ok(geo) => stats_for_geo.write().await.server_geo = Some(geo),
                Err(e) => eprintln!("fetch_geo error: {e}"),
            }
        });
        let mut ticker = interval(Duration::from_millis(self.interval));
        let app = Router::new()
            .route("/", get(show_index))
            .route("/api/stats", get(get_stats_api))
            .with_state(stats.clone());
        let listener = TcpListener::bind(&self.listen).await?;
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        println!("Real-time report available at http://{}", self.listen);
        tokio::select! {
            _ = signal::ctrl_c() => println!("\nExited."),
            _ = axum::serve(listener, app) => {},
            _ = async {
                loop {
                    ticker.tick().await;
                    let timestamp = Local::now();
                    let sent_at = Instant::now();
                    let command = if stats.clone().read().await.client_geo.is_none() {
                        1
                    } else {
                        0
                    };
                    if let Err(e) = socket.send_to(&[command], server_addr).await {
                        eprintln!("send to error: {e}");
                    }
                    let mut buf = [0u8; 32];
                    tokio::select! {
                        recv_res = socket.recv_from(&mut buf) => {
                            match recv_res {
                                Ok((len, addr)) if addr == server_addr => {
                                    if buf[0] == 1 {
                                        match parse_socket_addr(&buf[1..len]) {
                                            Ok(addr) => {
                                                let ip = addr.ip();
                                                let s = stats.clone();
                                                tokio::spawn(async move {
                                                    match fetch_geo(ip).await {
                                                        Ok(geo) => s.write().await.client_geo = Some(geo),
                                                        Err(e) => eprintln!("fetch_geo: {e}"),
                                                    }
                                                });

                                            },
                                            Err(e) => eprintln!("{e}"),

                                        };
                                    }
                                    stats.write().await.record(timestamp, Some(sent_at.elapsed()));
                                }
                                _ => stats.write().await.record(timestamp, None),
                            }
                        }
                        _ = sleep(Duration::from_secs(self.timeout)) => stats.write().await.record(timestamp, None),
                    }
                }
            } => {}
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    Cli::parse().run().await
}
