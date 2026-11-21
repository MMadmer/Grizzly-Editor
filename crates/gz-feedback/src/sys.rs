//! The machine as a report describes it: the system, the processor, the memory - never a name.

use serde_json::{Value, json};

/// The system, the processor and the memory.
#[must_use]
pub fn system() -> Value {
    let logical = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let mut out = json!({
        "arch": std::env::consts::ARCH,
        "logical_cpus": logical,
    });
    #[cfg(windows)]
    {
        let s = gz_win::system_info();
        out["os"] = json!(s.os);
        out["cpu"] = json!(s.cpu);
        out["ram_mb"] = json!(s.ram_mb);
    }
    out
}

/// What the editor's process holds of memory now.
#[must_use]
pub fn memory() -> Value {
    #[cfg(windows)]
    {
        let m = gz_win::process_memory();
        json!({ "working_set_mb": m.working_set_mb, "peak_working_set_mb": m.peak_working_set_mb, "private_mb": m.private_mb })
    }
    #[cfg(not(windows))]
    Value::Null
}
