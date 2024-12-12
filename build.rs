fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg=-Wl,-headerpad,0xFF");
    }
}
