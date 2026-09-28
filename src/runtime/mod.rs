pub mod executor;
pub mod loader;
pub mod mount;
pub mod selfextract;
pub mod vfs;

// 内存EXE执行（RunPE 进程挖坑）—— 仅 --features runpe 时编译，默认关闭（免杀软误报）
#[cfg(all(windows, feature = "runpe"))]
pub mod runpe;
#[cfg(all(windows, feature = "runpe"))]
pub use runpe::run_pe_memory;

pub use loader::RuntimeLoader;
pub use mount::{mount_vfs_to_drive, unmount, MountHandle};
pub use selfextract::{detect_self_extract, embedded_secunzip};
pub use vfs::VirtualFS;
