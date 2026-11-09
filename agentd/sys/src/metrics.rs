//! Hardware metrics for `bar.sys`: CPU, memory, disk, temperature from `/proc`, `statvfs` and `sysfs`. No sensor or not
//! Linux means `None`, never dummy values. GPU is the amdgpu driver's `gpu_busy_percent` in `sysfs`; NVML (NVIDIA) is not wired.

use std::path::Path;

/// One sample; percentages 0-100, temperature in degrees C.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    pub cpu: Option<f64>,
    pub gpu: Option<f64>,
    pub ram: Option<f64>,
    pub disk: Option<f64>,
    pub temp: Option<f64>,
}

/// Total and idle ticks from the first line of `/proc/stat`.
#[must_use]
pub fn parse_cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().next()?.strip_prefix("cpu ")?;
    let v: Vec<u64> = line
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal …
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some((v.iter().take(8).sum(), idle))
}

/// CPU load between two samples `(total, idle)`.
#[must_use]
pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> Option<f64> {
    let total = now.0.checked_sub(prev.0).filter(|t| *t > 0)?;
    let idle = now.1.saturating_sub(prev.1).min(total);
    Some(round1(100.0 * (total - idle) as f64 / total as f64))
}

/// Used memory, %: `1 - MemAvailable / MemTotal` from `/proc/meminfo`.
#[must_use]
pub fn parse_ram_percent(meminfo: &str) -> Option<f64> {
    let get = |k: &str| {
        meminfo.lines().find_map(|l| {
            l.strip_prefix(k)?
                .split_whitespace()
                .next()?
                .parse::<f64>()
                .ok()
        })
    };
    let (total, avail) = (get("MemTotal:")?, get("MemAvailable:")?);
    (total > 0.0).then(|| round1(100.0 * (1.0 - avail / total)))
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Highest `thermal` zone temperature, degrees C (values are in millidegrees).
#[must_use]
pub fn max_temp(thermal_dir: &Path) -> Option<f64> {
    let rd = std::fs::read_dir(thermal_dir).ok()?;
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("thermal_zone"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path().join("temp"))
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map(|m| m / 1000.0)
        .filter(|t| (-50.0..=150.0).contains(t))
        .fold(None, |a: Option<f64>, t| Some(a.map_or(t, |x| x.max(t))))
}

/// GPU load, %: the highest `gpu_busy_percent` among `card*/device` in the DRM directory (amdgpu driver). A card without the file
/// (Intel, NVIDIA without a `sysfs` sensor) gives `None`: no dummy values; a value outside 0-100 is discarded.
#[must_use]
pub fn gpu_percent(drm_dir: &Path) -> Option<f64> {
    let rd = std::fs::read_dir(drm_dir).ok()?;
    rd.flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            // `card0`, `card1`, ...; not `card0-HDMI-A-1` and not `renderD128`.
            n.strip_prefix("card")
                .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|e| {
            std::fs::read_to_string(e.path().join("device/gpu_busy_percent"))
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()
        })
        .filter(|v| (0.0..=100.0).contains(v))
        .fold(None, |a: Option<f64>, v| Some(a.map_or(v, |x| x.max(v))))
}

/// Used space of the filesystem at `path`, %.
#[must_use]
pub fn disk_percent(path: &Path) -> Option<f64> {
    let s = rustix::fs::statvfs(path).ok()?;
    let total = s.f_blocks as f64;
    (total > 0.0).then(|| round1(100.0 * (1.0 - s.f_bavail as f64 / total)))
}

/// Accumulates CPU ticks between calls.
#[derive(Default)]
pub struct Sampler {
    prev: Option<(u64, u64)>,
}

impl Sampler {
    /// Sample; the first call gives `cpu = None` (no previous value).
    pub fn sample(&mut self) -> Sample {
        let mut s = Sample {
            disk: disk_percent(Path::new("/")),
            ..Sample::default()
        };
        if !cfg!(target_os = "linux") {
            return s;
        }
        if let Some(now) = std::fs::read_to_string("/proc/stat")
            .ok()
            .as_deref()
            .and_then(parse_cpu_times)
        {
            s.cpu = self.prev.and_then(|p| cpu_percent(p, now));
            self.prev = Some(now);
        }
        s.ram = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .as_deref()
            .and_then(parse_ram_percent);
        s.temp = max_temp(Path::new("/sys/class/thermal"));
        s.gpu = gpu_percent(Path::new("/sys/class/drm"));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_between_samples() {
        let a = parse_cpu_times("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4").unwrap();
        let b = parse_cpu_times("cpu  200 0 200 750 150 0 0 0 0 0").unwrap();
        // total +300, idle +100 -> busy 200/300
        assert_eq!(cpu_percent(a, b), Some(66.7));
        assert_eq!(cpu_percent(b, b), None, "no growth means no value");
        assert!(parse_cpu_times("garbage").is_none());
    }

    #[test]
    fn ram_percent_from_meminfo() {
        let m = "MemTotal:       1000 kB\nMemFree: 1 kB\nMemAvailable:    250 kB\n";
        assert_eq!(parse_ram_percent(m), Some(75.0));
        assert!(parse_ram_percent("MemTotal: 0 kB\nMemAvailable: 0 kB").is_none());
        assert!(parse_ram_percent("empty").is_none());
    }

    #[test]
    fn temperature_takes_the_hottest_zone_and_ignores_garbage() {
        let d = tempfile::tempdir().unwrap();
        for (z, v) in [
            ("thermal_zone0", "41000"),
            ("thermal_zone1", "55500"),
            ("thermal_zone2", "-274000"),
            ("cooling_device0", "99000"),
        ] {
            std::fs::create_dir(d.path().join(z)).unwrap();
            std::fs::write(d.path().join(z).join("temp"), v).unwrap();
        }
        assert_eq!(max_temp(d.path()), Some(55.5));
        assert_eq!(max_temp(&d.path().join("missing")), None);
    }

    #[test]
    fn gpu_takes_the_busiest_card_and_ignores_garbage() {
        let d = tempfile::tempdir().unwrap();
        for (card, v) in [
            ("card0", Some("12")),
            ("card1", Some("47\n")),
            ("card2", Some("abc")),
            ("card3", None),
            ("renderD128", Some("99")),
            ("card0-HDMI-A-1", Some("99")),
        ] {
            let dev = d.path().join(card).join("device");
            std::fs::create_dir_all(&dev).unwrap();
            if let Some(v) = v {
                std::fs::write(dev.join("gpu_busy_percent"), v).unwrap();
            }
        }
        assert_eq!(gpu_percent(d.path()), Some(47.0));
        std::fs::write(d.path().join("card1/device/gpu_busy_percent"), "250").unwrap();
        assert_eq!(
            gpu_percent(d.path()),
            Some(12.0),
            "a value outside 0-100 is discarded"
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(gpu_percent(empty.path()), None);
        assert_eq!(gpu_percent(&d.path().join("missing")), None);
    }

    #[test]
    fn disk_is_a_percentage() {
        let p = disk_percent(Path::new("/")).unwrap();
        assert!((0.0..=100.0).contains(&p));
    }
}
