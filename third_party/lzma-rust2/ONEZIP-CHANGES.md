# OneZip 对 lzma-rust2 0.23.0 的修改

来源：crates.io 上的 `lzma-rust2` 0.23.0（<https://github.com/hasenbanck/lzma-rust2>），
许可 Apache-2.0（见 LICENSE）。只有 sevenz-rust2 用这个版本；OneZip 自己用的
0.24 不受影响。

## 多线程 LZMA2 编码器的进度与取消

原版 `Lzma2WriterMt` 一次能把"线程数 × 块大小"（16 线程时 256 MB 以上）的输入
读进内存再慢慢压缩。只按读取量统计进度时，进度会立刻跳到 100%，之后长时间
没有变化；此时也没有地方检查取消标志，取消或关闭程序要等压缩全部做完。

加了 `hooks` 模块（`src/enc/lzma2_writer_mt.rs`）：

- `hooks::set(progress, cancel)`：之后在本线程创建的多线程 LZMA2 / XZ 编码器，
  工作线程每压完约 1 MB 输入就调用一次 `progress`（参数是未压缩字节数）；
- 同时把 `cancel` 设为编码器的取消标志，工作线程在每 1 MB 之间检查，尽快停下。

涉及文件：`src/enc/lzma2_writer_mt.rs`、`src/xz/writer_mt.rs`（XZ 的工作线程原来
一次写完整块，改为分段写入）。

sevenz-rust2 在调用方线程上创建并驱动编码器，所以用线程局部变量传递，
不需要修改 sevenz-rust2。
