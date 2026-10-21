fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/app-icon.ico")
            .set("ProductName", "NDPI Viewer")
            .set("FileDescription", "NDPI Viewer")
            .set("OriginalFilename", "NDPIViewer.exe")
            .compile()
            .expect("Could not compile Windows application resources");
    }
}
