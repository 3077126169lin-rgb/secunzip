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

/// 请求密钥（普通用户：需已授权）
pub async fn request_key(server: &str, app_id: &str, user_id: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/key", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "user_id": user_id,
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

/// 申请临时权限
pub async fn request_access(
    server: &str,
    app_id: &str,
    user_id: &str,
    need_days: Option<i32>,
    message: Option<String>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/request", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "user_id": user_id,
            "need_days": need_days,
            "message": message,
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

/// 授权用户
pub async fn grant_user(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<String>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/grant", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
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
pub async fn approve_request(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<String>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/approve", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
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
