fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../memspark-ui/assets/memspark-icon.ico");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
