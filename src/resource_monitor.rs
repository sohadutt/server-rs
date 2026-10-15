use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub user_cpu_us: i64,
    pub system_cpu_us: i64,
    pub peak_rss_kb: i64,
    pub minor_faults: i64,
    pub major_faults: i64,
    pub voluntary_switches: i64,
    pub involuntary_switches: i64,
}

impl Snapshot {
    pub fn capture() -> Option<Self> {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage initializes the supplied rusage record on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: getrusage returned success and filled the record.
        let usage = unsafe { usage.assume_init() };
        let timeval_us =
            |value: libc::timeval| value.tv_sec as i64 * 1_000_000 + value.tv_usec as i64;
        #[cfg(target_os = "macos")]
        let peak_rss_kb = usage.ru_maxrss as i64 / 1024;
        #[cfg(not(target_os = "macos"))]
        let peak_rss_kb = usage.ru_maxrss as i64;
        Some(Self {
            user_cpu_us: timeval_us(usage.ru_utime),
            system_cpu_us: timeval_us(usage.ru_stime),
            peak_rss_kb,
            minor_faults: usage.ru_minflt as i64,
            major_faults: usage.ru_majflt as i64,
            voluntary_switches: usage.ru_nvcsw as i64,
            involuntary_switches: usage.ru_nivcsw as i64,
        })
    }
}

pub fn log_request(
    start: Option<Snapshot>,
    started: Instant,
    line: &str,
    bytes_in: usize,
    bytes_out: usize,
) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    if let Some(start) = start {
        if let Some(end) = Snapshot::capture() {
            log::debug!(
                "Request resources: request=\"{}\" wall={:.3}ms user_cpu={:.3}ms system_cpu={:.3}ms process_peak_rss={}KB minor_faults={} major_faults={} voluntary_context_switches={} involuntary_context_switches={} bytes_in={} bytes_out={}",
                sanitize(line),
                started.elapsed().as_secs_f64() * 1000.0,
                delta(end.user_cpu_us, start.user_cpu_us) as f64 / 1000.0,
                delta(end.system_cpu_us, start.system_cpu_us) as f64 / 1000.0,
                end.peak_rss_kb,
                delta(end.minor_faults, start.minor_faults),
                delta(end.major_faults, start.major_faults),
                delta(end.voluntary_switches, start.voluntary_switches),
                delta(end.involuntary_switches, start.involuntary_switches),
                bytes_in,
                bytes_out,
            );
            return;
        }
    }
    log::debug!(
        "Request resources: request=\"{}\" wall={:.3}ms bytes_in={} bytes_out={}",
        sanitize(line),
        started.elapsed().as_secs_f64() * 1000.0,
        bytes_in,
        bytes_out
    );
}

fn delta(end: i64, start: i64) -> i64 {
    end.saturating_sub(start).max(0)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .take(140)
        .map(|character| {
            if character == '"' || character == '\\' || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect()
}
