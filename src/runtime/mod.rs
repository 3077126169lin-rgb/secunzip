pub mod loader;
pub mod executor;
pub mod vfs;
pub mod mount;
pub mod selfextract;

// 内存EXE执行（RunPE 进程挖坑）—— 仅 --features runpe 时编译，默认关闭（免杀软误报）
#[cfg(all(windows, feature = "runpe"))]
pub mod runpe;
#[cfg(all(windows, feature = "runpe"))]
pub use runpe::run_pe_memory;

pub use loader::RuntimeLoader;
pub use vfs::VirtualFS;
pub use mount::{MountHandle, mount_vfs_to_drive, unmount};
pub use selfextract::{detect_self_extract, embedded_secunzip};
