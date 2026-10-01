//! Embeds the Windows icon and version resource into `ferrite.exe`. A no-op on
//! every other host: the icon reaches macOS through the `.app` bundle and Linux
//! through the `.desktop` entry (scripts/package.sh), not the binary.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/ferrite.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon/ferrite.ico");
        res.set("ProductName", "Ferrite");
        res.set("FileDescription", "Ferrite — an agentic browser");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not stop a build that only
            // loses its icon; say so loudly instead.
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
