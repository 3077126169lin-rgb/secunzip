use std::path::Path;
use std::collections::HashMap;
use crate::Result;

/// 执行模式
pub enum ExecuteMode {
    /// 落盘到临时目录
    TempDir,
    /// 纯内存（不落盘）
    Memory,
}

/// 执行器
pub struct Executor {
    mode: ExecuteMode,
}

impl Executor {
    pub fn new(mode: ExecuteMode) -> Self {
        Self { mode }
    }

    /// 执行文件
    pub fn execute(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        match self.mode {
            ExecuteMode::TempDir => self.execute_tempdir(files),
            ExecuteMode::Memory => self.execute_memory(files),
        }
    }

    /// 临时目录模式（落盘）
    fn execute_tempdir(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        let temp_dir = tempfile::tempdir()
            .map_err(|e| crate::SecUnzipError::Unpacking(format!("创建临时目录失败: {}", e)))?;

        for (name, content) in files {
            let file_path = temp_dir.path().join(name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, content)?;
        }

        open_path(temp_dir.path())?;

        println!("临时目录: {}", temp_dir.path().display());
        println!("按 Enter 退出...");
        wait_for_input();

        Ok(())
    }

    /// 内存模式（不落盘）
    fn execute_memory(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        // 检查文件类型
        let has_exe = files.iter().any(|(name, _)| {
            name.ends_with(".exe") || name.ends_with(".bat") || name.ends_with(".cmd")
        });

        let has_html = files.iter().any(|(name, _)| {
            name.ends_with(".html") || name.ends_with(".htm")
        });

        if has_exe {
            // 可执行文件：内存加载执行
            self.execute_memory_exe(files)?;
        } else if has_html {
            // HTML：内存中渲染
            self.execute_memory_html(files)?;
        } else {
            // 其他：提示用户
            self.execute_memory_view(files)?;
        }

        Ok(())
    }

    /// 内存执行EXE
    fn execute_memory_exe(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        // 找到主EXE
        let (exe_name, exe_data) = files.iter()
            .find(|(name, _)| name.ends_with(".exe"))
            .ok_or_else(|| crate::SecUnzipError::Unpacking("未找到EXE文件".into()))?;

        println!("内存执行: {}", exe_name);
        println!("大小: {} KB", exe_data.len() / 1024);

        // Windows: 使用 RunPE 技术在内存中加载
        #[cfg(windows)]
        {
            // TODO: 实现真正的 RunPE
            // 暂时使用临时文件 + 立即删除的方式
            let temp = std::env::temp_dir().join(format!("{}.exe", uuid::Uuid::new_v4()));
            std::fs::write(&temp, exe_data)?;
            
            // 启动进程
            let mut child = std::process::Command::new(&temp)
                .spawn()
                .map_err(|e| crate::SecUnzipError::Unpacking(format!("执行失败: {}", e)))?;

            // 立即删除文件（进程已加载到内存）
            let _ = std::fs::remove_file(&temp);

            println!("进程已启动 (PID: {})", child.id());
            println!("按 Enter 终止...");
            wait_for_input();

            // 终止进程
            let _ = child.kill();
        }

        #[cfg(not(windows))]
        {
            return Err(crate::SecUnzipError::Unpacking("内存执行暂不支持此平台".into()));
        }

        Ok(())
    }

    /// 内存渲染HTML
    fn execute_memory_html(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        let (html_name, _html_data) = files.iter()
            .find(|(name, _)| name.ends_with(".html") || name.ends_with(".htm"))
            .ok_or_else(|| crate::SecUnzipError::Unpacking("未找到HTML文件".into()))?;

        println!("内存渲染: {}", html_name);

        // 创建内存中的资源映射
        let mut resources: HashMap<String, Vec<u8>> = HashMap::new();
        for (name, data) in files {
            resources.insert(name.clone(), data.clone());
        }

        // TODO: 启动本地HTTP服务器，从内存提供资源
        // 暂时提示
        println!("资源文件数: {}", files.len());
        println!("TODO: 启动内存HTTP服务器");

        Ok(())
    }

    /// 内存查看其他文件
    fn execute_memory_view(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        println!("内存中的文件:");
        for (name, data) in files {
            println!("{} ({} KB)", name, data.len() / 1024);
        }
        println!();
        println!("纯内存模式：文件不会写入磁盘");
        println!("按 Enter 退出...");
        wait_for_input();

        Ok(())
    }
}

/// 打开路径
#[cfg(windows)]
fn open_path(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开失败: {}", e)))?;
    Ok(())
}

#[cfg(not(windows))]
fn open_path(path: &Path) -> Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开失败: {}", e)))?;
    Ok(())
}

/// 等待用户输入
fn wait_for_input() {
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
}
