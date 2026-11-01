// Windows hedefi: exe'ye DarkDrop simgesini ve sürüm bilgisini göm (Linux'tan çapraz derlemede de).
fn main() {
    println!("cargo:rerun-if-changed=assets/logo/darkdrop.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logo/darkdrop.ico");
        res.set("ProductName", "DarkDrop");
        res.set("FileDescription", "DarkDrop");
        if let Err(e) = res.compile() {
            println!("cargo:warning=simge gömülemedi: {e}");
        }
    }
}
