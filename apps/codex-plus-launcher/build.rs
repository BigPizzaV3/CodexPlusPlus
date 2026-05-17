fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../codex-plus-manager/src-tauri/icons/icon.ico");
        resource.compile().expect("compile launcher icon resource");
    }
}
