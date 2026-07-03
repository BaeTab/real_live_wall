//! System-state sampling (CPU / memory) exposed to shaders as reactive inputs.
//! Refreshed at most once per second, which is well above sysinfo's minimum CPU
//! sampling interval and keeps overhead negligible.

use std::time::{Duration, Instant};

use sysinfo::System;

pub struct Reactive {
    sys: System,
    last: Instant,
    cpu: f32,
    mem: f32,
}

impl Reactive {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_memory();
        Self {
            sys,
            // Force a refresh on the first poll.
            last: Instant::now() - Duration::from_secs(2),
            cpu: 0.0,
            mem: 0.0,
        }
    }

    /// Returns `(cpu_load, memory_used)`, both normalised to 0..1.
    pub fn poll(&mut self) -> (f32, f32) {
        if self.last.elapsed() >= Duration::from_millis(1000) {
            self.sys.refresh_cpu_usage();
            self.sys.refresh_memory();
            self.cpu = (self.sys.global_cpu_usage() / 100.0).clamp(0.0, 1.0);
            let total = self.sys.total_memory().max(1);
            self.mem = (self.sys.used_memory() as f32 / total as f32).clamp(0.0, 1.0);
            self.last = Instant::now();
        }
        (self.cpu, self.mem)
    }
}

impl Default for Reactive {
    fn default() -> Self {
        Self::new()
    }
}
