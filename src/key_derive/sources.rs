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
        return Err(crate::SecUnzipError::KeyDerivation(format!(
            "无效前缀长度: {}（必须 <= 32）",
            prefix
        )));
    }

    // prefix == 0 表示匹配全部 IPv4：此时不能写 `1u32 << 32`（debug 下 panic，
    // release 下移位计数按模 32 取，语义错误）。prefix 在 1..=32 时 `32 - prefix`
    // 最大为 31，移位安全。
    let mask: u32 = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let ip_u32 = u32::from(ip);
    let network_u32 = u32::from(network);

    Ok((ip_u32 & mask) == (network_u32 & mask))
}

#[cfg(test)]
mod tests {
    use super::ip_in_cidr;

    #[test]
    fn test_ip_in_cidr_prefix_zero_matches_all() {
        // prefix 0 曾经触发 `1u32 << 32` 溢出：debug 构建直接 panic，
        // release 构建按模 32 得到错误掩码。此用例正是为了覆盖该路径。
        assert!(ip_in_cidr("8.8.8.8", "0.0.0.0/0").unwrap());
        assert!(ip_in_cidr("192.168.1.1", "0.0.0.0/0").unwrap());
        assert!(ip_in_cidr("255.255.255.255", "10.0.0.0/0").unwrap());
    }

    #[test]
    fn test_ip_in_cidr_prefix_32_is_exact_host() {
        assert!(ip_in_cidr("203.0.113.7", "203.0.113.7/32").unwrap());
        assert!(!ip_in_cidr("203.0.113.8", "203.0.113.7/32").unwrap());
    }

    #[test]
    fn test_ip_in_cidr_middle_prefix() {
        // /24 掩码
        assert!(ip_in_cidr("192.168.1.255", "192.168.1.0/24").unwrap());
        assert!(!ip_in_cidr("192.168.2.1", "192.168.1.0/24").unwrap());
        // /8 掩码
        assert!(ip_in_cidr("10.255.1.2", "10.0.0.0/8").unwrap());
        assert!(!ip_in_cidr("11.0.0.1", "10.0.0.0/8").unwrap());
        // /31 边界
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0/31").unwrap());
        assert!(!ip_in_cidr("10.0.0.2", "10.0.0.0/31").unwrap());
    }

    #[test]
    fn test_ip_in_cidr_rejects_bad_input() {
        // 前缀越界
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0/33").is_err());
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0/999").is_err());
        // 非法 IP
        assert!(ip_in_cidr("not-an-ip", "10.0.0.0/8").is_err());
        assert!(ip_in_cidr("10.0.0.1", "not-a-net/8").is_err());
        // 畸形 CIDR
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0").is_err());
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0/8/9").is_err());
        assert!(ip_in_cidr("10.0.0.1", "10.0.0.0/").is_err());
    }
}
