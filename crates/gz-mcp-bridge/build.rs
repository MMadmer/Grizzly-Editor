//! The executable's icon: the editor's, so grizzly-mcp.exe reads as part of it.

fn main() {
    let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/grizzly-editor-icon.ico");
    if let Err(e) = gz_ico::embed(&ico) {
        println!("cargo:warning=the icon: {e}");
        std::process::exit(1);
    }
}
