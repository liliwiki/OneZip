fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("编译界面失败");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/onezip.ico");
        res.set("FileDescription", "OneZip");
        res.set("ProductName", "OneZip");
        res.compile().expect("嵌入图标失败");
    }
}
