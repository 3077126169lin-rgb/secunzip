//! 内存虚拟文件系统（Virtual File System）
//!
//! 将解密解压后的文件装载为内存中的只读文件树，行为上等价于挂载一个只读镜像：
//! 目录可列举、文件可读取，但不可写、不可改。它不是 ISO，也不经过任何镜像文件或
//! 磁盘设备，内容只存在内存中，关闭程序后自动消失，防止复制。

use crate::Result;
use std::collections::BTreeMap;

/// 虚拟文件系统节点
#[derive(Debug, Clone)]
pub enum VfsNode {
    /// 目录（子节点按名称排序）
    Dir(BTreeMap<String, VfsNode>),
    /// 文件（名称, 内容）
    File(Vec<u8>),
}

/// 只读内存文件系统
pub struct VirtualFS {
    root: VfsNode,
    total_size: usize,
    file_count: usize,
}

/// 文件条目信息（用于列表展示）
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// 路径（相对根目录，用 / 分隔）
    pub path: String,
    /// 文件名
    pub name: String,
    /// 是否目录
    pub is_dir: bool,
    /// 大小（目录为 0）
    pub size: usize,
}

impl VirtualFS {
    /// 从文件列表构建虚拟文件系统
    pub fn from_files(files: Vec<(String, Vec<u8>)>) -> Self {
        let mut root = VfsNode::Dir(BTreeMap::new());
        let mut total_size = 0usize;
        let mut file_count = 0usize;

        for (path, content) in files {
            total_size += content.len();
            file_count += 1;
            insert_path(&mut root, &path, content);
        }

        Self {
            root,
            total_size,
            file_count,
        }
    }

    /// 总大小
    pub fn total_size(&self) -> usize {
        self.total_size
    }

    /// 文件数量
    pub fn file_count(&self) -> usize {
        self.file_count
    }

    /// 列出指定目录下的条目
    pub fn list_dir(&self, dir_path: &str) -> Vec<FileEntry> {
        let node = self.resolve(dir_path);
        match node {
            Some(VfsNode::Dir(children)) => children
                .iter()
                .map(|(name, child)| {
                    let full_path = if dir_path.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", dir_path.trim_end_matches('/'), name)
                    };
                    let (is_dir, size) = match child {
                        VfsNode::Dir(_) => (true, 0),
                        VfsNode::File(data) => (false, data.len()),
                    };
                    FileEntry {
                        path: full_path,
                        name: name.clone(),
                        is_dir,
                        size,
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// 读取文件内容
    pub fn read_file(&self, path: &str) -> Result<&[u8]> {
        match self.resolve(path) {
            Some(VfsNode::File(data)) => Ok(data),
            Some(VfsNode::Dir(_)) => {
                Err(crate::SecUnzipError::Unpacking(format!("{} 是目录", path)))
            }
            None => Err(crate::SecUnzipError::Unpacking(format!(
                "文件不存在: {}",
                path
            ))),
        }
    }

    /// 读取文本文件内容（UTF-8，无效字节替换）
    pub fn read_text(&self, path: &str) -> Result<String> {
        let data = self.read_file(path)?;
        Ok(String::from_utf8_lossy(data).into_owned())
    }

    /// 判断是否可能是文本文件（根据扩展名）
    pub fn is_text_file(path: &str) -> bool {
        let text_exts = [
            "txt",
            "md",
            "rs",
            "py",
            "js",
            "ts",
            "jsx",
            "tsx",
            "html",
            "htm",
            "css",
            "json",
            "xml",
            "yml",
            "yaml",
            "toml",
            "ini",
            "cfg",
            "conf",
            "log",
            "csv",
            "sql",
            "sh",
            "bat",
            "ps1",
            "c",
            "h",
            "cpp",
            "hpp",
            "java",
            "go",
            "rb",
            "php",
            "lua",
            "swift",
            "kt",
            "gradle",
            "cmake",
            "gitignore",
            "dockerfile",
            "makefile",
            "env",
        ];
        match path.rsplit_once('.') {
            Some((_, ext)) => text_exts.contains(&ext.to_lowercase().as_str()),
            // 无扩展名的可能是 README、LICENSE 等
            None => {
                let name = path.rsplit('/').next().unwrap_or(path).to_uppercase();
                matches!(
                    name.as_str(),
                    "README" | "LICENSE" | "CHANGELOG" | "MAKEFILE" | "DOCKERFILE"
                )
            }
        }
    }

    /// 是否为图片文件
    pub fn is_image_file(path: &str) -> bool {
        let img_exts = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "svg"];
        match path.rsplit_once('.') {
            Some((_, ext)) => img_exts.contains(&ext.to_lowercase().as_str()),
            None => false,
        }
    }

    /// 解析路径到节点
    fn resolve(&self, path: &str) -> Option<&VfsNode> {
        let mut current = &self.root;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            match current {
                VfsNode::Dir(children) => {
                    current = children.get(part)?;
                }
                VfsNode::File(_) => return None,
            }
        }
        Some(current)
    }
}

/// 递归插入路径到文件树
fn insert_path(root: &mut VfsNode, path: &str, content: Vec<u8>) {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    insert_parts(root, &parts, content);
}

/// 按路径段递归插入
fn insert_parts(node: &mut VfsNode, parts: &[&str], content: Vec<u8>) {
    if parts.is_empty() {
        return;
    }
    // 非目录节点无法插入子项
    let VfsNode::Dir(children) = node else { return };

    if parts.len() == 1 {
        children.insert(parts[0].to_string(), VfsNode::File(content));
    } else {
        let entry = children
            .entry(parts[0].to_string())
            .or_insert_with(|| VfsNode::Dir(BTreeMap::new()));
        insert_parts(entry, &parts[1..], content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_files() -> Vec<(String, Vec<u8>)> {
        vec![
            ("readme.txt".to_string(), b"Hello VFS".to_vec()),
            ("src/main.rs".to_string(), b"fn main() {}".to_vec()),
            ("src/lib.rs".to_string(), b"pub mod vfs;".to_vec()),
            ("src/runtime/mod.rs".to_string(), b"pub mod vfs;".to_vec()),
            ("data.json".to_string(), br#"{"key":"value"}"#.to_vec()),
        ]
    }

    #[test]
    fn test_build_and_list_root() {
        let vfs = VirtualFS::from_files(sample_files());
        let entries = vfs.list_dir("");
        // 根目录：readme.txt, src/, data.json
        assert_eq!(entries.len(), 3);
        assert_eq!(vfs.file_count(), 5);
    }

    #[test]
    fn test_list_subdir() {
        let vfs = VirtualFS::from_files(sample_files());
        let entries = vfs.list_dir("src");
        // src: main.rs, lib.rs, runtime/
        assert_eq!(entries.len(), 3);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"main.rs"));
        assert!(names.contains(&"lib.rs"));
        assert!(names.contains(&"runtime"));
    }

    #[test]
    fn test_read_file() {
        let vfs = VirtualFS::from_files(sample_files());
        let content = vfs.read_text("readme.txt").unwrap();
        assert_eq!(content, "Hello VFS");
        let content = vfs.read_text("src/main.rs").unwrap();
        assert_eq!(content, "fn main() {}");
    }

    #[test]
    fn test_read_missing_file() {
        let vfs = VirtualFS::from_files(sample_files());
        assert!(vfs.read_file("nope.txt").is_err());
    }

    #[test]
    fn test_read_dir_as_file_fails() {
        let vfs = VirtualFS::from_files(sample_files());
        assert!(vfs.read_file("src").is_err());
    }

    #[test]
    fn test_is_text_file() {
        assert!(VirtualFS::is_text_file("readme.txt"));
        assert!(VirtualFS::is_text_file("src/main.rs"));
        assert!(VirtualFS::is_text_file("data.json"));
        assert!(VirtualFS::is_text_file("README"));
        assert!(!VirtualFS::is_text_file("image.png"));
        assert!(!VirtualFS::is_text_file("archive.zip"));
    }

    #[test]
    fn test_is_image_file() {
        assert!(VirtualFS::is_image_file("photo.png"));
        assert!(VirtualFS::is_image_file("a/b/photo.JPEG"));
        assert!(!VirtualFS::is_image_file("readme.txt"));
    }

    #[test]
    fn test_nested_deep() {
        let vfs = VirtualFS::from_files(sample_files());
        let content = vfs.read_text("src/runtime/mod.rs").unwrap();
        assert_eq!(content, "pub mod vfs;");
    }

    #[test]
    fn test_empty_vfs() {
        let vfs = VirtualFS::from_files(vec![]);
        assert_eq!(vfs.file_count(), 0);
        assert!(vfs.list_dir("").is_empty());
    }

    #[test]
    fn test_total_size() {
        let vfs = VirtualFS::from_files(sample_files());
        let expected: usize = sample_files().iter().map(|(_, c)| c.len()).sum();
        assert_eq!(vfs.total_size(), expected);
    }
}
