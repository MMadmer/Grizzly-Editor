//! The hub's upload key a build of the editor carries for its reports. It is never in the repository:
//! `GRIZZLY_REPORT_UPLOAD_TOKEN` at build time or the ignored file `report_token.local` beside this
//! script gives it; without one the build sends no report and says so (`Build.ps1 -Package` refuses
//! such a release). A QA run's key is another variable, read at run time: docs/REPORTS.md.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=GRIZZLY_REPORT_UPLOAD_TOKEN");
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let local = Path::new(&dir).join("report_token.local");
    // a path cargo is told to watch must exist, or the script runs again at every build: without the
    // file the crate's folder is watched, which a file made there later changes
    println!("cargo:rerun-if-changed=build.rs");
    if local.is_file() {
        println!("cargo:rerun-if-changed={}", local.display());
    } else {
        println!("cargo:rerun-if-changed={dir}");
    }
    let token = std::env::var("GRIZZLY_REPORT_UPLOAD_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| std::fs::read_to_string(&local).ok())
        .map(|t| t.trim().to_owned())
        .filter(|t| t.len() >= 24 && t.bytes().all(|b| b.is_ascii_graphic()))
        .unwrap_or_default();
    // kept out of a plain look at the program, the way the game keeps its own
    let hidden: String = token.bytes().map(|b| format!("{:02x}", b ^ 0xA7)).collect();
    println!("cargo:rustc-env=GRIZZLY_REPORT_UPLOAD_HIDDEN={hidden}");
}
