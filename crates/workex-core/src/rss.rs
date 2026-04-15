//! OS-level RSS (Resident Set Size) memory measurement.
//!
//! Returns the real physical memory used by this process,
//! not estimates or struct sizes.

/// Get the current process RSS in bytes.
/// This is the actual physical RAM the OS has allocated to this process.
pub fn get_rss_bytes() -> usize {
    platform::get_rss_bytes()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub fn get_rss_bytes() -> usize {
        unsafe {
            let handle = GetCurrentProcess();
            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(handle, &mut counters, cb) != 0 {
                counters.WorkingSetSize
            } else {
                0
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub fn get_rss_bytes() -> usize {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(val) = line.strip_prefix("VmRSS:") {
                    let val = val.trim();
                    if let Some(kb_str) = val.strip_suffix("kB").or_else(|| val.strip_suffix("KB")) {
                        if let Ok(kb) = kb_str.trim().parse::<usize>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
        0
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn get_rss_bytes() -> usize {
        0 // Unsupported platform
    }
}

/// Measure RSS delta around a closure.
/// Calls the closure, then returns (result, rss_before, rss_after, delta).
pub fn measure_rss_delta<F, R>(f: F) -> (R, usize, usize, usize)
where
    F: FnOnce() -> R,
{
    let before = get_rss_bytes();
    let result = f();
    let after = get_rss_bytes();
    let delta = after.saturating_sub(before);
    (result, before, after, delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_returns_nonzero() {
        let rss = get_rss_bytes();
        assert!(rss > 0, "RSS should be > 0, got {rss}");
        // Should be at least a few MB for any Rust process
        assert!(
            rss > 1024 * 1024,
            "RSS should be > 1MB, got {} bytes",
            rss
        );
    }

    #[test]
    fn rss_delta_detects_allocation() {
        let (data, _before, _after, delta) = measure_rss_delta(|| {
            // Allocate ~10MB
            let v: Vec<u8> = vec![42u8; 10 * 1024 * 1024];
            std::hint::black_box(&v);
            v
        });
        // Keep data alive for measurement
        std::hint::black_box(&data);

        // Delta should be at least a few MB (may not be exactly 10MB due to OS paging)
        assert!(
            delta > 1024 * 1024,
            "RSS delta should detect large alloc, got {} bytes",
            delta
        );
    }
}
