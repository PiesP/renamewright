#![forbid(unsafe_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=resources/windows.rc");
    println!("cargo:rerun-if-changed=resources/app-icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/windows.rc", embed_resource::NONE)
            .manifest_required()?;
    }
    Ok(())
}
