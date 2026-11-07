//! Exe-icoon. Alleen op Windows; het venstericoon staat apart in main.rs.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/sheetdiff.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../../assets/icon/sheetdiff.ico")
        .compile()
        .expect("exe-icoon niet in te bouwen");
}
