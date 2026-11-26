fn main() {
    // 界面文字是中文，英文译文来自仓库根目录的 i18n/en.po（见 onezip-i18n）。
    println!("cargo:rerun-if-changed=../../i18n/en.po");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .with_bundled_translations(onezip_i18n::bundle_for_slint());
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
