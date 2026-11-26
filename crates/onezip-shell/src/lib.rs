//! onezip_shell.dll：Windows 11 资源管理器第一层右键菜单。
//!
//! 实现 `IExplorerCommand`：一个"OneZip"根菜单，下面是各个子命令。
//! 子命令只负责把选中的路径交给同目录下的 OneZip.exe，实际工作都在 OneZip.exe 里做，
//! 这样资源管理器（或承载本 DLL 的 dllhost）里不会跑任何重活，崩了也不影响资源管理器。
//!
//! 这个 DLL 需要通过稀疏包（packaging/sparse/AppxManifest.xml）注册，
//! 资源管理器才会把它放进第一层菜单。

/// 本扩展的 COM 类 ID，与 AppxManifest.xml 里的一致。
///
/// 只注册这一个第一层菜单项：同一个应用注册两项时，Win11 会把它们再套进一层
/// 没有图标的同名菜单里（OneZip ▸ OneZip ▸ …），反而更深。
pub const CLSID_TEXT: &str = "3847becd-6a0c-43a7-b898-e1f2c38be845";

// 菜单的内容（各项、顺序、文字、参数）在 onezip-menu 里，和传统注册表菜单共用。
pub use onezip_menu::{Command, Entry, HASH_ITEMS, MENU, Show, visible};

#[cfg(windows)]
mod com;
