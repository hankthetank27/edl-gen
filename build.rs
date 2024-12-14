fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.lock");
    // set hook for Sparkle update framework
    let target = std::env::var("TARGET").expect("TARGET environment variable not set");
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg=-Wl,-headerpad,0xFF");
    }
}
