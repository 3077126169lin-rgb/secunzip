use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 服务器连通性检测（GET 根路径；任何 HTTP 响应视为在线，连接失败/超时视为离线）
pub async fn ping_server(server: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    else {
        return false;
    };
    client.get(server).send().await.is_ok()
}

#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    pub success: bool,
    pub message: String,
    pub key: Option<String>,
    pub requests: Option<Vec<RequestInfo>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestInfo {
    pub user_id: String,
    pub need_days: Option<i32>,
    pub message: Option<String>,
    pub created_at: String,
}

/// 请求密钥（普通用户：需已授权 + 本人预设的口令）。
///
/// 口令由客户端明文传输、在服务端校验；这里绝不自行哈希。
/// `password` 为空时整个字段不发送，服务端会以「口令缺失」拒绝并给出提示。
pub async fn request_key(
    server: &str,
    app_id: &str,
    user_id: &str,
    password: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({
        "app_id": app_id,
        "user_id": user_id,
    });
    if let Some(pwd) = password.filter(|p| !p.is_empty()) {
        body["password"] = serde_json::json!(pwd);
    }
    let resp = client
        .post(format!("{}/api/key", server))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    if data.success {
        Ok(data.key.unwrap_or_default())
    } else {
        Err(data.message)
    }
}

/// 管理员取密钥（凭 secret，免申请）
pub async fn request_key_admin(server: &str, app_id: &str, secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/key", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    if data.success {
        Ok(data.key.unwrap_or_default())
    } else {
        Err(data.message)
    }
}

/// 申请临时权限（申请人自行设定口令，用于日后取密钥）
pub async fn request_access(
    server: &str,
    app_id: &str,
    user_id: &str,
    need_days: Option<i32>,
    message: Option<String>,
    password: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/request", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "user_id": user_id,
            "need_days": need_days,
            "message": message,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    Ok(data.message)
}

/// 授权用户（管理员为对方设定取密钥用的口令）
pub async fn grant_user(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<String>,
    password: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/grant", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    Ok(data.message)
}

/// 吊销用户
pub async fn revoke_user(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/revoke", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    Ok(data.message)
}

/// 获取待审批列表（需该文件的 secret；服务端按 app_id 过滤）
pub async fn list_requests(
    server: &str,
    app_id: &str,
    secret: &str,
) -> Result<Vec<RequestInfo>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/requests", server))
        .json(&serde_json::json!({ "app_id": app_id, "secret": secret }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    if data.success {
        Ok(data.requests.unwrap_or_default())
    } else {
        Err(data.message)
    }
}

/// 审批通过
///
/// `password` 为空表示不覆盖：整个字段不发送，服务端沿用申请人在申请时设定的口令。
pub async fn approve_request(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<String>,
    password: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({
        "app_id": app_id,
        "secret": secret,
        "user_id": user_id,
        "expires_at": expires_at,
    });
    if let Some(pwd) = password.filter(|p| !p.is_empty()) {
        body["password"] = serde_json::json!(pwd);
    }
    let resp = client
        .post(format!("{}/api/approve", server))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    Ok(data.message)
}

/// 审批拒绝
pub async fn deny_request(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/deny", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    Ok(data.message)
}
