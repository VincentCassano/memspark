fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile Slint UI");
    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/memspark-icon.ico");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
