fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/MonitorDDC.ico");
        resource.set("FileDescription", "MonitorDDC 显示器调节工具");
        resource.set("ProductName", "MonitorDDC");
        resource.set("OriginalFilename", "MonitorDDC.exe");
        resource
            .compile()
            .expect("failed to embed Windows executable resources");
    }
}
