fn main() {
    // 界面文字是中文，英文译文来自仓库根目录的 i18n/en.po（见 onezip-i18n）。
    println!("cargo:rerun-if-changed=../../i18n/en.po");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .with_bundled_translations(onezip_i18n::bundle_for_slint());
    slint_build::compile_with_config("ui/setup.slint", config).expect("编译界面失败");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/onezip.ico");
        res.set("FileDescription", "OneZip 安装程序");
        res.set("ProductName", "OneZip");
        // 文件名带 "Setup" 的程序，Windows 默认会当成安装程序要求管理员权限。
        // OneZip 只装给当前用户，必须以普通权限运行，否则会装进管理员账户。
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>"#,
        );
        res.compile().expect("嵌入资源失败");
    }
}
