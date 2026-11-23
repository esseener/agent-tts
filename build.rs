fn main() {
    println!("cargo:rerun-if-changed=assets/AgentVoice.ico");
    let manifest = format!(
        r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0"><assemblyIdentity version="{}.0" processorArchitecture="amd64" name="AgentVoice.Desktop" type="win32"/><trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security></trustInfo><application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings><dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware><activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage></windowsSettings></application></assembly>"#,
        env!("CARGO_PKG_VERSION")
    );
    winresource::WindowsResource::new()
        .set_icon("assets/AgentVoice.ico")
        .set("ProductName", "Agent Voice")
        .set(
            "FileDescription",
            "Agent Voice — native agent audio console",
        )
        .set("FileVersion", env!("CARGO_PKG_VERSION"))
        .set("ProductVersion", env!("CARGO_PKG_VERSION"))
        .set_manifest(&manifest)
        .compile()
        .expect("Windows icon and version resources");
}
