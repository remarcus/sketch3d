fn main() {
    println!("cargo:rerun-if-changed=assets/icons/sketch3d.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/sketch3d.ico")
            .compile()
            .expect("compile Windows application icon resource");
    }
}
