# OneZip 对 Peel 的修改

来源：<https://github.com/agouin/peel> 的提交 `77d2e34b6138cc6279a5e0d1a64271b07faa0219`（v0.8.0），
许可 MIT OR Apache-2.0，与 OneZip 相同。只拷了构建需要的部分（`src/`、`Cargo.toml`、
许可证、NOTICE、README），去掉了两个示例程序的条目。

## WinZip AE-2 加密条目不再比较 CRC

WinZip AES 的 AE-2 格式规定条目的 CRC 字段写 0，完整性由 HMAC 保证（WinZip、7-Zip
以及 Rust 的 `zip` crate 对小文件都会这样写）。原版 `ZipSink::end_entry` 一律拿算出的
CRC 和 0 比，于是这类条目必定报 "CRC-32 mismatch"，带密码的 zip 解不出来。

改为：流水线遇到 AE-2 条目时调用新增的 `ZipSink::skip_crc_check`，该条目只靠 AES
解密器里的 HMAC 校验。AE-1 与不加密的条目照旧比较 CRC。

涉及文件：`src/sink/zip.rs`、`src/download/zip_pipeline.rs`。

## 7z：文件名加密（加密的头）也能解

7-Zip 的"加密文件名"会把记录文件列表的头也加密（`EncodedHeader` 的文件夹带 AES 编码器）。
原版解这个头时没有传密码，于是报 "no password source was configured"，即使给了密码也一样。

改为：头所在的文件夹是加密的就先取密码再解；没有校验值可用，解码或解析失败一律报
"密码不对"（`PasswordIncorrect`），让前端重新要密码。

涉及文件：`src/download/sevenz_pipeline.rs`。
