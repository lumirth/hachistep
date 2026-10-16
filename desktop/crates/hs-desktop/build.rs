fn main() {
    #[cfg(target_os = "windows")]
    {
        winresource::WindowsResource::new()
            .set_icon("../../../assets/icons/windows/icon.ico")
            .set("ProductName", "hachistep")
            .set("FileDescription", "Pokéwalker emulator")
            .compile()
            .expect("embed Windows icon");
    }
    println!("cargo:rerun-if-changed=../../../assets/icons/windows/icon.ico");
}
