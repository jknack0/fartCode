//! Resource monitor (E1-09): CPU/memory sampling via sysinfo, shown as a
//! palette panel. Disabled by default (`resourceMonitor.enabled = false`).

use std::sync::Mutex;

use crate::Error;

/// A single machine-health sample.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSample {
    pub cpu_percent: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
}

/// sysinfo's `System` is heavy to construct; reuse one across samples (it
/// refreshes per call).
static SYSTEM: Mutex<Option<sysinfo::System>> = Mutex::new(None);

/// Samples CPU + memory (lazy sysinfo instance, refreshed per call).
pub fn sample() -> Result<ResourceSample, Error> {
    let mut guard = SYSTEM
        .lock()
        .map_err(|_| Error::Internal("resource-monitor mutex poisoned".into()))?;
    if guard.is_none() {
        let mut system = sysinfo::System::new();
        // sysinfo needs two refreshes for a meaningful first CPU sample —
        // warm up at init so the panel never shows 0% on its first tick.
        system.refresh_cpu_usage();
        *guard = Some(system);
    }
    let system = guard.as_mut().unwrap();
    system.refresh_cpu_usage();
    system.refresh_memory();

    Ok(ResourceSample {
        cpu_percent: system.global_cpu_info().cpu_usage(),
        mem_used_mb: system.used_memory() / (1024 * 1024),
        mem_total_mb: system.total_memory() / (1024 * 1024),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_cpu_and_memory() {
        let s = sample().unwrap();
        assert!(
            (0.0..=100.0).contains(&s.cpu_percent),
            "cpu: {}",
            s.cpu_percent
        );
        assert!(s.mem_total_mb > 0, "total memory known");
        assert!(s.mem_used_mb <= s.mem_total_mb, "used <= total: {s:?}");
    }
}
