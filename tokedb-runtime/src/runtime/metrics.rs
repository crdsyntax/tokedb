use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    
    pub cpu_percent: f64,
    
    pub memory_bytes: u64,
    
    pub memory_limit_bytes: Option<u64>,
    
    pub pids: u64,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::ResourceUsage;

    const SAMPLE_MS: u64 = 250;

    struct Snapshot {
        rss: u64,
        cpu_ticks: u64,
        pids: u64,
    }

    
    
    
    
    pub fn collect_usage(root_pid: u32, memory_limit: Option<u64>) -> ResourceUsage {
        let first = snapshot(root_pid);
        std::thread::sleep(std::time::Duration::from_millis(SAMPLE_MS));
        let second = snapshot(root_pid);

        
        
        const CLK_TCK: f64 = 100.0;
        let ncpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64;
        let elapsed = SAMPLE_MS as f64 / 1000.0;
        let cpu_ticks = (second.cpu_ticks as f64 - first.cpu_ticks as f64).max(0.0);
        let cpu_percent = (cpu_ticks / CLK_TCK) / elapsed / ncpus * 100.0;

        ResourceUsage {
            cpu_percent,
            memory_bytes: second.rss,
            memory_limit_bytes: memory_limit,
            pids: second.pids,
        }
    }

    fn snapshot(root_pid: u32) -> Snapshot {
        let mut rss = 0u64;
        let mut cpu_ticks = 0u64;
        let mut pids = 0u64;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let pid = match entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) {
                    Some(pid) => pid,
                    None => continue,
                };
                if !is_in_container(pid, root_pid) {
                    continue;
                }
                pids += 1;
                if let Some(value) = read_rss(pid) {
                    rss += value;
                }
                if let Some(ticks) = read_cpu_ticks(pid) {
                    cpu_ticks += ticks;
                }
            }
        }
        Snapshot {
            rss,
            cpu_ticks,
            pids,
        }
    }

    
    fn is_in_container(pid: u32, root_pid: u32) -> bool {
        if pid == root_pid {
            return true;
        }
        let mut current = pid;
        for _ in 0..32 {
            match read_ppid(current) {
                Some(ppid) if ppid == root_pid => return true,
                Some(ppid) if ppid == 0 => return false,
                Some(ppid) => current = ppid,
                None => return false,
            }
        }
        false
    }

    fn read_ppid(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        
        let rest = stat.split(')').nth(1)?;
        let parts: Vec<&str> = rest.split_whitespace().collect();
        
        let ppid = parts.get(1)?.parse::<u32>().ok()?;
        Some(ppid)
    }

    fn read_cpu_ticks(pid: u32) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = stat.split(')').nth(1)?;
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let utime = parts.get(11)?.parse::<u64>().ok()?;
        let stime = parts.get(12)?.parse::<u64>().ok()?;
        Some(utime + stime)
    }

    fn read_rss(pid: u32) -> Option<u64> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }
}

#[cfg(target_os = "linux")]
pub use linux::collect_usage;

#[cfg(not(target_os = "linux"))]
pub fn collect_usage(_root_pid: u32, _memory_limit: Option<u64>) -> ResourceUsage {
    ResourceUsage::default()
}
