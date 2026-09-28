use crate::core::KeySource;
use crate::Result;
use chrono::Local;

/// 获取密钥派生源数据
pub fn get_source_data(source: &KeySource) -> Result<Vec<u8>> {
    match source {
        KeySource::MachineGuid => get_machine_guid(),
        KeySource::LocalIp => get_local_ip(),
        KeySource::CurrentDate => get_current_date(),
        KeySource::DomainUser => get_domain_user(),
        KeySource::Salt(s) => Ok(s.clone()),
        KeySource::Literal(s) => Ok(s.as_bytes().to_vec()),
        KeySource::SteamId => {
            // TODO: 实现 SteamID 获取
            Err(crate::SecUnzipError::KeyDerivation(
                "SteamID 获取尚未实现".into(),
            ))
        }
    }
}

/// 获取 Windows MachineGUID
#[cfg(windows)]
fn get_machine_guid() -> Result<Vec<u8>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let crypto = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法读取注册表: {}", e)))?;

    let guid: String = crypto
        .get_value("MachineGuid")
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法获取 MachineGUID: {}", e)))?;

    Ok(guid.into_bytes())
}

#[cfg(not(windows))]
fn get_machine_guid() -> Result<Vec<u8>> {
    // Linux: /etc/machine-id
    std::fs::read("/etc/machine-id")
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法读取 machine-id: {}", e)))
}

/// 获取本机内网IP
pub fn get_local_ip() -> Result<Vec<u8>> {
    use std::net::UdpSocket;

    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法创建socket: {}", e)))?;

    // 连接到外部地址来获取本机IP（不会真正发送数据）
    socket
        .connect("8.8.8.8:80")
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法获取本机IP: {}", e)))?;

    let addr = socket
        .local_addr()
        .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("无法获取本地地址: {}", e)))?;

    Ok(addr.ip().to_string().into_bytes())
}

/// 获取当前日期 YYYYMMDD
fn get_current_date() -> Result<Vec<u8>> {
    let date = Local::now().format("%Y%m%d").to_string();
    Ok(date.into_bytes())
}

/// 获取域名/用户名
#[cfg(windows)]
fn get_domain_user() -> Result<Vec<u8>> {
    // 使用环境变量方式获取用户名，避免复杂的Windows API调用
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .map(|u| u.into_bytes())
        .map_err(|_| crate::SecUnzipError::KeyDerivation("无法获取用户名".into()))
}

#[cfg(not(windows))]
fn get_domain_user() -> Result<Vec<u8>> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .map(|u| u.into_bytes())
        .map_err(|_| crate::SecUnzipError::KeyDerivation("无法获取用户名".into()))
}

/// 检查IP是否在CIDR范围内
pub fn ip_in_cidr(ip: &str, cidr: &str) -> Result<bool> {
    use std::net::Ipv4Addr;

    let ip: Ipv4Addr = ip
        .parse()
        .map_err(|_| crate::SecUnzipError::KeyDerivation(format!("无效IP: {}", ip)))?;

    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return Err(crate::SecUnzipError::KeyDerivation(format!(
            "无效CIDR: {}",
            cidr
        )));
    }

    let network: Ipv4Addr = parts[0]
        .parse()
        .map_err(|_| crate::SecUnzipError::KeyDerivation(format!("无效网络地址: {}", parts[0])))?;

    let prefix: u32 = parts[1]
        .parse()
        .map_err(|_| crate::SecUnzipError::KeyDerivation(format!("无效前缀长度: {}", parts[1])))?;

    if prefix > 32 {
        return Err(crate::SecUnzipError::KeyDerivation(
            "前缀长度必须 <= 32".into(),
        ));
    }

    let mask = !((1u32 << (32 - prefix)) - 1);
    let ip_u32 = u32::from(ip);
    let network_u32 = u32::from(network);

    Ok((ip_u32 & mask) == (network_u32 & mask))
}
