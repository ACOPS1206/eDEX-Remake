use maxminddb::{Reader, geoip2};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant, SystemTime},
};

pub type Position = (f64, f64);

#[derive(Default)]
pub struct NetworkView {
    pub delay_ms: Option<u64>,
    pub endpoint: Option<Position>,
    pub peers: Vec<Position>,
    pub geo_available: bool,
}

pub struct Watcher {
    database: PathBuf,
    modified: Option<SystemTime>,
    reader: Option<Reader<Vec<u8>>>,
    positions: HashMap<IpAddr, Option<Position>>,
    observed: HashMap<IpAddr, Instant>,
    endpoint: Option<Position>,
    checked_endpoint: Option<Instant>,
    local: Option<Ipv4Addr>,
    target: String,
}

impl Watcher {
    pub fn new(database: PathBuf, target: String) -> Self {
        Self {
            database,
            target,
            modified: None,
            reader: None,
            positions: HashMap::new(),
            observed: HashMap::new(),
            endpoint: None,
            checked_endpoint: None,
            local: None,
        }
    }

    pub fn sample(&mut self, local: Option<Ipv4Addr>, observed: Vec<IpAddr>) -> NetworkView {
        let metadata = self
            .database
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        if metadata != self.modified {
            self.modified = metadata;
            self.reader = metadata.and_then(|_| Reader::open_readfile(&self.database).ok());
            self.positions.clear();
            self.checked_endpoint = None;
        }
        if local != self.local {
            self.local = local;
            self.checked_endpoint = None;
            self.endpoint = None;
        }
        let mut view = NetworkView {
            geo_available: self.reader.is_some(),
            ..NetworkView::default()
        };
        if let Some(local) = local.filter(|ip| !ip.is_loopback()) {
            view.delay_ms = latency(&self.target, local);
        }
        let now = Instant::now();
        self.observed
            .retain(|_, seen| now.duration_since(*seen) < Duration::from_secs(18));
        for address in observed {
            if self.observed.len() < 192 {
                self.observed.insert(address, now);
            }
        }
        if let Some(reader) = &self.reader {
            let mut addresses = active_peers();
            for address in self
                .observed
                .keys()
                .take(300_usize.saturating_sub(addresses.len()))
            {
                addresses.insert(*address);
            }
            self.positions
                .retain(|address, _| addresses.contains(address));
            view.peers = addresses
                .into_iter()
                .filter_map(|ip| {
                    *self
                        .positions
                        .entry(ip)
                        .or_insert_with(|| lookup(reader, ip))
                })
                .collect();
            if let Some(local) = local.filter(|ip| !ip.is_loopback()) {
                if self
                    .checked_endpoint
                    .is_none_or(|when| when.elapsed() > Duration::from_secs(60))
                {
                    let public = if is_public(IpAddr::V4(local)) {
                        Some(IpAddr::V4(local))
                    } else {
                        external_address(local)
                    };
                    self.endpoint = public.and_then(|address| lookup(reader, address));
                    self.checked_endpoint = Some(Instant::now());
                }
                view.endpoint = self.endpoint;
            }
        }
        view
    }
}

pub fn database(configured: Option<&str>) -> PathBuf {
    if let Some(path) = configured.filter(|path| !path.is_empty()) {
        return path.into();
    }
    let config = crate::theme::config_path();
    [
        config.join("geoIPcache/GeoLite2-City.mmdb"),
        config.join("GeoLite2-City.mmdb"),
        PathBuf::from("/var/lib/GeoIP/GeoLite2-City.mmdb"),
        PathBuf::from("/usr/share/GeoIP/GeoLite2-City.mmdb"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(cache_path)
}

pub fn cache_path() -> PathBuf {
    crate::theme::config_path().join("geoIPcache/GeoLite2-City.mmdb")
}

fn external_address(local: Ipv4Addr) -> Option<IpAddr> {
    let result = Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "3",
            "--max-filesize",
            "4096",
            "--interface",
            &local.to_string(),
            "https://myexternalip.com/json",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !result.status.success() {
        return None;
    }
    let response: serde_json::Value = serde_json::from_slice(&result.stdout).ok()?;
    let address = response["ip"].as_str()?.parse().ok()?;
    is_public(address).then_some(address)
}

fn latency(target: &str, local: Ipv4Addr) -> Option<u64> {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    use std::net::{SocketAddr, ToSocketAddrs};
    let started = Instant::now();
    for address in (target, 80)
        .to_socket_addrs()
        .ok()?
        .filter(SocketAddr::is_ipv4)
        .take(2)
    {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).ok()?;
        socket.bind(&SocketAddr::from((local, 0)).into()).ok()?;
        let timeout = Duration::from_millis(1900).checked_sub(started.elapsed())?;
        if socket
            .connect_timeout(&SockAddr::from(address), timeout)
            .is_ok()
        {
            return Some(started.elapsed().as_millis() as u64);
        }
    }
    None
}

fn lookup(database: &Reader<Vec<u8>>, address: IpAddr) -> Option<Position> {
    let record = database.lookup::<geoip2::City<'_>>(address).ok()??;
    let location = record.location?;
    let (lat, lon) = (location.latitude?, location.longitude?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000 && segments[..2] != [0x2001, 0x0db8]
        }
    }
}

pub fn terminal_addresses(text: &str) -> Vec<IpAddr> {
    let mut seen = HashSet::new();
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '.')
        .filter_map(|word| {
            word.trim_end_matches('.')
                .parse::<Ipv4Addr>()
                .ok()
                .map(IpAddr::V4)
        })
        .filter(|address| is_public(*address) && seen.insert(*address))
        .take(32)
        .collect()
}

fn active_peers() -> HashSet<IpAddr> {
    [("/proc/net/tcp", false), ("/proc/net/tcp6", true)]
        .into_iter()
        .filter_map(|(file, ipv6)| fs::read_to_string(file).ok().map(|text| (text, ipv6)))
        .flat_map(|(text, ipv6)| connection_addresses(&text, ipv6).collect::<Vec<_>>())
        .take(300)
        .collect()
}

fn connection_addresses(text: &str, ipv6: bool) -> impl Iterator<Item = IpAddr> + '_ {
    text.lines().skip(1).filter_map(move |line| {
        let mut parts = line.split_ascii_whitespace();
        let remote = parts.nth(2)?.split_once(':')?.0;
        if parts.next()? != "01" {
            return None;
        }
        let ip = if ipv6 {
            if remote.len() != 32 || !remote.is_ascii() {
                return None;
            }
            let mut data = [0; 16];
            for (chunk, offset) in data.chunks_exact_mut(4).zip((0..32).step_by(8)) {
                chunk.copy_from_slice(
                    &u32::from_str_radix(&remote[offset..offset + 8], 16)
                        .ok()?
                        .to_ne_bytes(),
                );
            }
            let ip = Ipv6Addr::from(data);
            ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4)
        } else {
            IpAddr::V4(Ipv4Addr::from(
                u32::from_str_radix(remote, 16).ok()?.to_ne_bytes(),
            ))
        };
        is_public(ip).then_some(ip)
    })
}

pub fn download() -> Result<PathBuf, String> {
    const URL: &str = "https://raw.githubusercontent.com/GitSquared/node-geolite2-redist/master/redist/GeoLite2-City.tar.gz";
    const CHECKSUM: &str = "https://raw.githubusercontent.com/GitSquared/node-geolite2-redist/master/redist/GeoLite2-City.mmdb.sha384";
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    let destination = cache_path();
    let parent = destination.parent().ok_or("GeoIP cache path is invalid")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".geoip-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&temporary).map_err(|error| error.to_string())?;
    let result = (|| {
        let archive = temporary.join("city.tar.gz");
        let curl = Command::new("curl")
            .args([
                "-fLsS",
                "--max-time",
                "100",
                "--max-filesize",
                "100000000",
                "--proto",
                "=https",
                "-o",
            ])
            .arg(&archive)
            .arg(URL)
            .output()
            .map_err(|error| error.to_string())?;
        if !curl.status.success() {
            return Err(String::from_utf8_lossy(&curl.stderr).trim().to_owned());
        }
        let database = temporary.join("GeoLite2-City.mmdb");
        let mut output = fs::File::create(&database).map_err(|error| error.to_string())?;
        let mut process = Command::new("tar")
            .args(["--wildcards", "-xOf"])
            .arg(&archive)
            .arg("*/GeoLite2-City.mmdb")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut stdout = process.stdout.take().ok_or("Cannot read the archive")?;
        let mut bounded = io::Read::take(&mut stdout, 250_000_001);
        let bytes = io::copy(&mut bounded, &mut output).map_err(|error| error.to_string());
        drop(stdout);
        let status = process.wait().map_err(|error| error.to_string())?;
        if bytes? >= 250_000_001 || !status.success() {
            return Err("GeoLite2 archive is invalid or too large".to_owned());
        }
        let remote = Command::new("curl")
            .args([
                "-fLsS",
                "--max-time",
                "15",
                "--max-filesize",
                "256",
                "--proto",
                "=https",
                CHECKSUM,
            ])
            .output()
            .map_err(|error| error.to_string())?;
        let local = Command::new("sha384sum")
            .arg(&database)
            .output()
            .map_err(|error| error.to_string())?;
        let expected = String::from_utf8_lossy(&remote.stdout);
        let expected = expected.trim();
        if !remote.status.success()
            || !local.status.success()
            || expected.len() != 96
            || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
            || String::from_utf8_lossy(&local.stdout)
                .split_ascii_whitespace()
                .next()
                != Some(expected)
        {
            return Err("GeoLite2 checksum verification failed".into());
        }
        let reader = Reader::open_readfile(&database).map_err(|error| error.to_string())?;
        if !reader.metadata.database_type.contains("City") {
            return Err("Expected a GeoLite2 City database".into());
        }
        fs::rename(&database, &destination).map_err(|error| error.to_string())?;
        Ok(destination.clone())
    })();
    let _ = fs::remove_dir_all(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_unique_public_ipv4_addresses_in_terminal_output() {
        let text = "PING 81.2.69.142 (81.2.69.142): hello from 8.8.8.8. No 127.0.0.1, bad999.2.3.4 or 192.168.0.1.";
        assert_eq!(
            terminal_addresses(text),
            ["81.2.69.142", "8.8.8.8"].map(|ip| ip.parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn filters_nonpublic_and_nonestablished_tcp_peers() {
        let data = "sl local_address rem_address st\n0: 00000000:0000 08080808:0016 01\n1: 00000000:0000 0100007F:0016 01\n2: 00000000:0000 01010101:0016 06\n";
        assert_eq!(
            connection_addresses(data, false).collect::<Vec<_>>(),
            vec!["8.8.8.8".parse::<IpAddr>().unwrap()]
        );
        let data = "sl local_address rem_address st\n0: 00000000000000000000000000000000:0016 00470626000000000000000011110000:0016 01\n";
        assert!(connection_addresses(data, true).next().is_some());
    }

    #[test]
    fn looks_up_city_coordinates_when_fixture_is_provided() {
        if let Some(path) = std::env::var_os("EDEX_GEOIP_TEST_DB") {
            let reader = Reader::open_readfile(&path).unwrap();
            let position = lookup(&reader, "81.2.69.142".parse().unwrap()).unwrap();
            assert!((position.0 - 51.5).abs() < 0.1);
            let mut watcher = Watcher::new(path.into(), "1.1.1.1".into());
            let from_terminal = terminal_addresses("found 81.2.69.142");
            let mapped = watcher.sample(None, from_terminal);
            assert!(mapped.geo_available && mapped.peers.contains(&position));
        }
    }
}
