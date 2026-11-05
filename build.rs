fn main() {
    println!("cargo:rerun-if-changed=assets/icon/rippy.ico");

    // GPUI takes the window and taskbar icon from the executable's first icon resource.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon/rippy.ico");
        resource.compile().expect("failed to embed the Windows icon");
    }
}
