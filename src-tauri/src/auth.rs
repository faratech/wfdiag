use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use reqwest::header::SET_COOKIE;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthToken {
    pub token: String,
    pub username: String,
    pub user_id: u32,
    pub expires_at: u64,
    pub permissions: Vec<String>,
    pub xf_cookies: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    pub success: bool,
    pub token: Option<String>,
    pub username: Option<String>,
    pub user_id: Option<u32>,
    pub expires_at: Option<u64>,
    pub permissions: Option<Vec<String>>,
    pub xf_cookies: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthState {
    token: Option<AuthToken>,
    xf_cookies: Option<String>,
}

impl AuthState {
    pub fn new() -> Self {
        Self { 
            token: None,
            xf_cookies: None,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        if let Some(token) = &self.token {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            return now < token.expires_at;
        }
        false
    }

    pub fn set_token(&mut self, token: AuthToken) {
        self.xf_cookies = token.xf_cookies.clone();
        self.token = Some(token);
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        self.xf_cookies = None;
    }

    pub fn get_token(&self) -> Option<&AuthToken> {
        self.token.as_ref()
    }
    
    pub fn get_cookies(&self) -> Option<&String> {
        self.xf_cookies.as_ref()
    }
}

#[tauri::command]
pub async fn authenticate(
    username: String,
    password: String,
    state: tauri::State<'_, std::sync::Mutex<AuthState>>,
) -> Result<AuthResponse, String> {
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;
    
    let login_url = "https://windowsforum.com/login/login";
    
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("login", username.clone());
    form_data.insert("password", password);
    form_data.insert("remember", "1".to_string());
    
    let response = client
        .post(login_url)
        .form(&form_data)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| format!("Failed to connect to WindowsForum: {}", e))?;
    
    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|c| c.starts_with("xf_"))
        .map(|c| c.split(';').next().unwrap_or(c))
        .collect::<Vec<_>>()
        .join("; ");
    
    if cookies.is_empty() {
        return Err("Authentication failed: No session cookies received".to_string());
    }
    
    let verify_response = client
        .post("https://windowsforum.com/chat.php")
        .header("Cookie", &cookies)
        .json(&serde_json::json!({"action": "getUserData"}))
        .send()
        .await
        .map_err(|e| format!("Failed to verify session: {}", e))?;
    
    if !verify_response.status().is_success() {
        return Err("Authentication failed: Session verification failed".to_string());
    }
    
    let user_data: serde_json::Value = verify_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse user data: {}", e))?;
    
    let user_id = user_data["user_id"]
        .as_u64()
        .unwrap_or(0) as u32;
    
    let username_verified = user_data["name"]
        .as_str()
        .unwrap_or(&username)
        .to_string();
    
    if user_id == 0 || user_data["user_id"].as_str().unwrap_or("").starts_with("guest_") {
        return Err("Authentication failed: Invalid credentials".to_string());
    }
    
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() + 86400;
    
    let auth_token = AuthToken {
        token: format!("wf_{}", user_id),
        username: username_verified.clone(),
        user_id,
        expires_at,
        permissions: vec!["ai_analysis".to_string()],
        xf_cookies: Some(cookies.clone()),
    };
    
    let mut state = state.lock().unwrap();
    state.set_token(auth_token.clone());
    
    Ok(AuthResponse {
        success: true,
        token: Some(auth_token.token),
        username: Some(username_verified),
        user_id: Some(user_id),
        expires_at: Some(expires_at),
        permissions: Some(vec!["ai_analysis".to_string()]),
        xf_cookies: Some(cookies),
        error: None,
    })
}

#[tauri::command]
pub async fn logout(state: tauri::State<'_, std::sync::Mutex<AuthState>>) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    state.clear_token();
    Ok(())
}

#[tauri::command]
pub async fn check_auth_status(
    state: tauri::State<'_, std::sync::Mutex<AuthState>>,
) -> Result<bool, String> {
    let state = state.lock().unwrap();
    Ok(state.is_authenticated())
}

#[tauri::command]
pub async fn get_auth_info(
    state: tauri::State<'_, std::sync::Mutex<AuthState>>,
) -> Result<Option<AuthToken>, String> {
    let state = state.lock().unwrap();
    Ok(state.get_token().cloned())
}

#[tauri::command]
pub async fn get_session_cookies(
    state: tauri::State<'_, std::sync::Mutex<AuthState>>,
) -> Result<Option<String>, String> {
    let state = state.lock().unwrap();
    Ok(state.get_cookies().cloned())
}

