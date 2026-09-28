# Assets

打包用的运行时占位资源。

## runtime_stub.exe

11 字节的占位文件。

打包黑盒 EXE 时，[`src/packer/builder.rs`](../src/packer/builder.rs) 优先使用**打包工具自身的二进制**
作为 runner；只有取不到自身路径时才回退到这个占位文件（通过 `include_bytes!` 在编译期嵌入）。

因此它必须存在于仓库中（否则编译失败），但不参与正常产物的运行。
