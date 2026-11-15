use std::path::{Path, PathBuf};
use std::{collections::HashMap, ffi::CStr, fs, mem::MaybeUninit, net::Ipv4Addr, time::Instant};
use sysinfo::{Components, ProcessRefreshKind, ProcessesToUpdate, System};

pub const PROCESS_LIMIT: usize = 5;

#[derive(Default)]
pub struct Stats {
    pub cpu: f64,
    pub cpu_groups: [f64; 2],
    pub cpu_count: usize,
    pub processor: String,
    pub cpu_speed: u64,
    pub cpu_max: u64,
    pub tasks: usize,
    pub battery: Option<u32>,
    pub temperature: Option<f32>,
    pub memory: u64,
    pub memory_total: u64,
    pub memory_reclaimable: u64,
    pub swap: u64,
    pub swap_total: u64,
    pub uptime: u64,
    pub root: Option<(u64, u64)>,
    pub network: Option<Network>,
    pub processes: Option<Vec<Process>>,
}

#[derive(Clone)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub memory_percent: f64,
    pub user: String,
    pub state: String,
    pub started: u64,
    pub runtime: u64,
}

pub struct Network {
    pub name: String,
    pub rx: f64,
    pub tx: f64,
    pub total_rx: u64,
    pub total_tx: u64,
    pub address: Option<Ipv4Addr>,
}

pub struct Sampler {
    system: System,
    network_counters: HashMap<String, (u64, u64)>,
    temperatures: Components,
    root: Option<(u64, u64)>,
    interface: Option<String>,
    last_sample: Instant,
    tick: u64,
    group_threads: bool,
    battery: Option<u32>,
    max_frequency: u64,
}

impl Sampler {
    pub fn new(interface: Option<String>, group_threads: bool) -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        system.refresh_cpu_frequency();
        Self {
            system,
            network_counters: read_network_counters(),
            temperatures: Components::new(),
            root: None,
            interface,
            last_sample: Instant::now(),
            tick: 0,
            group_threads,
            battery: None,
            max_frequency: fs::read_to_string(
                "/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq",
            )
            .ok()
            .and_then(|freq| freq.trim().parse::<u64>().ok())
            .unwrap_or(0)
                / 1000,
        }
    }

    pub fn sample(&mut self) -> Stats {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sample).as_secs_f64();
        self.last_sample = now;
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let processes = if self.tick.is_multiple_of(3) {
            let kind = ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(sysinfo::UpdateKind::OnlyIfNotSet);
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                if self.group_threads {
                    kind.without_tasks()
                } else {
                    kind
                },
            );
            let accounts = fs::read_to_string("/etc/passwd").unwrap_or_default();
            let names: HashMap<_, _> = accounts
                .lines()
                .filter_map(|entry| {
                    let mut fields = entry.split(':');
                    let name = fields.next()?;
                    let uid = fields.nth(1)?;
                    Some((uid, name))
                })
                .collect();
            let mut processes: Vec<_> = self
                .system
                .processes()
                .values()
                .map(|process| {
                    let uid = process
                        .user_id()
                        .map(|uid| (**uid).to_string())
                        .unwrap_or_default();
                    let user = names.get(uid.as_str()).copied().unwrap_or(&uid).to_owned();
                    Process {
                        pid: process.pid().as_u32(),
                        name: process.name().to_string_lossy().into_owned(),
                        cpu: process.cpu_usage(),
                        memory: process.memory(),
                        memory_percent: 0.0,
                        user,
                        state: process.status().to_string(),
                        started: process.start_time(),
                        runtime: process.run_time(),
                    }
                })
                .collect();
            if self.group_threads {
                processes.sort_unstable_by_key(|process| process.pid);
                let mut grouped = HashMap::<String, Process>::new();
                for process in processes {
                    if let Some(previous) = grouped.get_mut(&process.name) {
                        previous.cpu += process.cpu;
                        previous.memory = previous.memory.saturating_add(process.memory);
                    } else {
                        grouped.insert(process.name.clone(), process);
                    }
                }
                processes = grouped.into_values().collect();
            }
            let memory_total = self.system.total_memory();
            for process in &mut processes {
                process.memory_percent = percent(process.memory, memory_total);
            }
            processes.sort_unstable_by(|a, b| {
                b.cpu
                    .total_cmp(&a.cpu)
                    .then_with(|| b.memory.cmp(&a.memory))
                    .then_with(|| a.pid.cmp(&b.pid))
            });
            Some(processes)
        } else {
            None
        };

        if self.tick.is_multiple_of(5) {
            self.temperatures.refresh(true);
            self.system.refresh_cpu_frequency();
            self.battery = battery_capacity();
        }
        if self.tick.is_multiple_of(10) {
            self.root = mount_usage(Path::new("/"));
        }

        let temperature = self
            .temperatures
            .iter()
            .filter(|component| {
                let label = component.label().to_ascii_lowercase();
                ["coretemp", "k10temp", "cpu", "package", "tctl", "tdie"]
                    .iter()
                    .any(|key| label.contains(key))
            })
            .filter_map(|component| component.temperature().filter(|value| value.is_finite()))
            .max_by(f32::total_cmp);

        let counters = read_network_counters();
        let delta = |name: &str| match (counters.get(name), self.network_counters.get(name)) {
            (Some(current), Some(previous)) => (
                current.0.saturating_sub(previous.0),
                current.1.saturating_sub(previous.1),
            ),
            _ => (0, 0),
        };
        let default_interface = if self.interface.is_none() {
            fs::read_to_string("/proc/net/route")
                .ok()
                .and_then(|routes| default_route(&routes).map(str::to_owned))
        } else {
            None
        };
        let network = self
            .interface
            .as_deref()
            .or(default_interface.as_deref())
            .filter(|name| counters.contains_key(*name))
            .or_else(|| {
                if self.interface.is_some() {
                    return None;
                }
                counters
                    .keys()
                    .map(String::as_str)
                    .filter(|name| *name != "lo")
                    .max_by(|a, b| {
                        let (rx_a, tx_a) = delta(a);
                        let (rx_b, tx_b) = delta(b);
                        rx_a.saturating_add(tx_a)
                            .cmp(&rx_b.saturating_add(tx_b))
                            .then_with(|| b.cmp(a))
                    })
            })
            .map(|name| {
                let (rx, tx) = delta(name);
                let (total_rx, total_tx) = counters[name];
                Network {
                    name: name.to_owned(),
                    rx: per_second(rx, elapsed),
                    tx: per_second(tx, elapsed),
                    total_rx,
                    total_tx,
                    address: interface_ipv4(name),
                }
            });
        self.network_counters = counters;

        let cpus = self.system.cpus();
        let (first, second) = cpus.split_at(cpus.len().div_ceil(2));
        let average = |group: &[sysinfo::Cpu]| {
            group
                .iter()
                .map(|cpu| f64::from(cpu.cpu_usage()))
                .sum::<f64>()
                / group.len().max(1) as f64
        };
        self.tick += 1;
        Stats {
            cpu_groups: [average(first), average(second)],
            cpu_count: cpus.len(),
            processor: cpus
                .first()
                .map(|cpu| cpu.brand().to_owned())
                .unwrap_or_default(),
            cpu_speed: cpus.iter().map(sysinfo::Cpu::frequency).sum::<u64>()
                / cpus.len().max(1) as u64,
            cpu_max: self
                .max_frequency
                .max(cpus.iter().map(sysinfo::Cpu::frequency).max().unwrap_or(0)),
            tasks: self.system.processes().len(),
            battery: self.battery,
            cpu: f64::from(self.system.global_cpu_usage()),
            temperature,
            memory: self.system.used_memory(),
            memory_total: self.system.total_memory(),
            memory_reclaimable: self
                .system
                .available_memory()
                .saturating_sub(self.system.free_memory()),
            swap: self.system.used_swap(),
            swap_total: self.system.total_swap(),
            uptime: System::uptime(),
            root: self.root,
            network,
            processes,
        }
    }
}

fn interface_ipv4(interface: &str) -> Option<Ipv4Addr> {
    let mut list = std::ptr::null_mut();
    // The C library retains ownership of the returned list until freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut current = list;
    let mut address = None;
    unsafe {
        while let Some(entry) = current.as_ref() {
            if !entry.ifa_name.is_null()
                && !entry.ifa_addr.is_null()
                && (*entry.ifa_addr).sa_family as i32 == libc::AF_INET
                && CStr::from_ptr(entry.ifa_name).to_bytes() == interface.as_bytes()
            {
                let inet = &*entry.ifa_addr.cast::<libc::sockaddr_in>();
                address = Some(Ipv4Addr::from(inet.sin_addr.s_addr.to_ne_bytes()));
                break;
            }
            current = entry.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    address
}

fn battery_capacity() -> Option<u32> {
    let capacities: Vec<_> = fs::read_dir("/sys/class/power_supply")
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            fs::read_to_string(path.join("type")).is_ok_and(|value| value.trim() == "Battery")
        })
        .filter_map(|path| {
            fs::read_to_string(path.join("capacity"))
                .ok()?
                .trim()
                .parse::<u32>()
                .ok()
        })
        .collect();
    (!capacities.is_empty()).then(|| capacities.iter().sum::<u32>() / capacities.len() as u32)
}

pub fn machine_label(name: &str) -> String {
    fs::read_to_string(format!("/sys/class/dmi/id/{name}"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .trim()
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "—".into())
}

pub fn chassis_label() -> String {
    match machine_label("chassis_type").as_str() {
        "3" | "4" | "5" | "6" | "7" => "Desktop",
        "8" | "9" | "10" | "14" => "Notebook",
        "13" => "All-in-One",
        "17" | "23" => "Server",
        "30" => "Tablet",
        "31" | "32" => "Convertible",
        _ => "—",
    }
    .into()
}

fn read_network_counters() -> HashMap<String, (u64, u64)> {
    fs::read_to_string("/proc/net/dev")
        .map(|contents| parse_network_counters(&contents))
        .unwrap_or_default()
}

fn parse_network_counters(contents: &str) -> HashMap<String, (u64, u64)> {
    contents
        .lines()
        .filter_map(|line| {
            let (name, fields) = line.rsplit_once(':')?;
            let mut fields = fields.split_whitespace();
            let rx = fields.next()?.parse().ok()?;
            let tx = fields.nth(7)?.parse().ok()?;
            Some((name.trim().to_owned(), (rx, tx)))
        })
        .collect()
}

pub struct Mount {
    pub path: PathBuf,
    pub source: String,
    pub kind: String,
}

pub fn mounts() -> Vec<Mount> {
    let entries = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut mounts: Vec<_> = entries
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let path = mount_path(left.split_ascii_whitespace().nth(4)?);
            let mut fields = right.split_ascii_whitespace();
            let kind = fields.next()?.to_owned();
            let source = fields.next()?.to_owned();
            let visible = path == Path::new("/")
                || source.starts_with("/dev/")
                || path.starts_with("/mnt")
                || path.starts_with("/media")
                || path.starts_with("/run/media")
                || path.starts_with("/home")
                || matches!(kind.as_str(), "nfs" | "nfs4" | "cifs" | "fuse.sshfs");
            visible.then_some(Mount { path, source, kind })
        })
        .collect();
    mounts.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    mounts.dedup_by(|a, b| a.path == b.path);
    mounts
}

fn mount_path(path: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = path.as_bytes();
    let mut path = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes
            .get(index..index + 4)
            .filter(|p| p[0] == b'\\' && p[1..].iter().all(|byte| (b'0'..=b'7').contains(byte)));
        if let Some(code) = escape {
            path.push(((code[1] - b'0') << 6) | ((code[2] - b'0') << 3) | (code[3] - b'0'));
            index += 4;
        } else {
            path.push(bytes[index]);
            index += 1;
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(path))
}

pub fn mount_usage(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut buffer = MaybeUninit::<libc::statvfs>::uninit();
    // A successful statvfs call fully initializes the returned filesystem data.
    if unsafe { libc::statvfs(path.as_ptr(), buffer.as_mut_ptr()) } != 0 {
        return None;
    }
    let fs = unsafe { buffer.assume_init() };
    // On macOS statvfs::f_blocks is u32, whereas Linux uses u64.
    // Normalize all inputs before multiplying so both targets compile and
    // large volumes do not accidentally overflow 32-bit intermediates.
    let blocks = u64::from(fs.f_blocks);
    let free = u64::from(fs.f_bfree);
    let block_size = u64::from(fs.f_frsize);
    let total = blocks.saturating_mul(block_size);
    let used = blocks.saturating_sub(free).saturating_mul(block_size);
    Some((used, total))
}

fn per_second(bytes: u64, elapsed: f64) -> f64 {
    if elapsed >= 0.5 {
        bytes as f64 / elapsed
    } else {
        0.0
    }
}

fn default_route(routes: &str) -> Option<&str> {
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let interface = fields.next()?;
            let destination = fields.next()?;
            let _gateway = fields.next()?;
            let flags = u32::from_str_radix(fields.next()?, 16).ok()?;
            let metric = fields.nth(2)?.parse::<u32>().ok()?;
            (destination == "00000000" && flags & 1 != 0).then_some((interface, metric))
        })
        .min_by_key(|(_, metric)| *metric)
        .map(|(interface, _)| interface)
}

pub fn quantity(bytes: f64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

pub fn percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * used as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_lowest_metric_active_default_route() {
        let routes = "Iface Destination Gateway Flags RefCnt Use Metric Mask\n\
                      eth0 00000000 0 0003 0 0 200 0\n\
                      wlan0 00000000 0 0003 0 0 100 0\n\
                      eth1 00000000 0 0000 0 0 1 0\n\
                      eth2 00000001 0 0001 0 0 1 FFFFFFFF";
        assert_eq!(default_route(routes), Some("wlan0"));
        assert_eq!(default_route("Iface Destination\neth0 broken"), None);
    }

    #[test]
    fn reads_receive_and_transmit_columns() {
        let counters = parse_network_counters(
            "Inter-| Receive | Transmit\n eth0: 1200 2 0 0 0 0 0 0 1300 3 0 0 0 0 0 0\n bad: 10",
        );
        assert_eq!(counters.len(), 1);
        assert_eq!(counters.get("eth0"), Some(&(1200, 1300)));
    }

    #[test]
    fn formats_binary_units_and_handles_missing_capacity() {
        assert_eq!(quantity(1536.0), "1.5 KiB");
        assert_eq!(quantity(0.0), "0 B");
        assert_eq!(percent(3, 12), 25.0);
        assert_eq!(percent(0, 0), 0.0);
    }
}
