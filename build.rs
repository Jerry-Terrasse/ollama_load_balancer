use std::io;

#[cfg(target_os = "windows")]
fn main() -> io::Result<()> {
    use winresource::WindowsResource;

    WindowsResource::new()
        // This path can be absolute, or relative to your crate root.
        .set_icon("doc/logo/logo.ico")
        .compile()?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() -> io::Result<()> {
    // Do nothing when not on Windows
    Ok(())
}
