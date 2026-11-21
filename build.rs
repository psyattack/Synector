fn main() {
    // Version resources are derived from Cargo.toml by winres; track the inputs this script uses
    // so a version bump or a replaced icon rebuilds the executable's resources.
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/app.ico");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("ProductName", "Synector");
        res.set("FileDescription", "Synector");
        res.set("CompanyName", "psyattack");
        res.set("LegalCopyright", "psyattack");
        res.set("OriginalFilename", "Synector.exe");
        res.set("InternalName", "Synector");
        res.set_manifest(r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
        <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
</compatibility>
<application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
        <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
        <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
</application>
</assembly>
"#);
        res.compile().unwrap_or_else(|error| {
            panic!("failed to compile the Windows resource file (icon, VERSIONINFO, manifest): {error}")
        });
    }
}
