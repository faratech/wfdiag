use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use url::Url;

// OAuth2 Configuration
const OAUTH_AUTHORIZE_URL: &str = "https://windowsforum.com/oauth/authorize";
const OAUTH_TOKEN_URL: &str = "https://windowsforum.com/oauth/token";
const OAUTH_USERINFO_URL: &str = "https://windowsforum.com/oauth/userinfo";
const CLIENT_ID: &str = "wfdiag-tauri";
const REDIRECT_URI: &str = "http://localhost:9420/callback";
const SCOPES: &str = "read user:email";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    pub id: u32,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthState {
    pub token: Option<OAuthToken>,
    pub user_info: Option<OAuthUserInfo>,
    pub pkce_verifier: Option<String>,
    pub state: Option<String>,
}

impl OAuthState {
    pub fn new() -> Self {
        Self {
            token: None,
            user_info: None,
            pkce_verifier: None,
            state: None,
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

    pub fn needs_refresh(&self) -> bool {
        if let Some(token) = &self.token {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            // Refresh if token expires in less than 5 minutes
            return token.expires_at.saturating_sub(now) < 300;
        }
        false
    }
}

// PKCE (Proof Key for Code Exchange) implementation
fn generate_random_string(length: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn generate_pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(result)
}

#[tauri::command]
pub async fn oauth_start_flow(
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<String, String> {
    // Generate PKCE verifier and challenge
    let pkce_verifier = generate_random_string(128);
    let pkce_challenge = generate_pkce_challenge(&pkce_verifier);
    
    // Generate state for CSRF protection
    let oauth_state = generate_random_string(32);
    
    // Store verifier and state
    {
        let mut oauth = state.lock().unwrap();
        oauth.pkce_verifier = Some(pkce_verifier.clone());
        oauth.state = Some(oauth_state.clone());
    }
    
    // Build authorization URL
    let mut url = Url::parse(OAUTH_AUTHORIZE_URL)
        .map_err(|e| format!("Failed to parse URL: {}", e))?;
    
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &pkce_challenge)
        .append_pair("code_challenge_method", "S256");
    
    Ok(url.to_string())
}

#[tauri::command]
pub async fn oauth_handle_callback(
    code: String,
    callback_state: String,
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<OAuthToken, String> {
    // Verify state to prevent CSRF
    let pkce_verifier = {
        let oauth = state.lock().unwrap();
        
        if oauth.state.as_ref() != Some(&callback_state) {
            return Err("Invalid state parameter - possible CSRF attack".to_string());
        }
        
        oauth.pkce_verifier.clone()
            .ok_or_else(|| "No PKCE verifier found".to_string())?
    };
    
    // Exchange authorization code for tokens
    let client = reqwest::Client::new();
    
    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", &code);
    params.insert("client_id", CLIENT_ID);
    params.insert("redirect_uri", REDIRECT_URI);
    params.insert("code_verifier", &pkce_verifier);
    
    let response = client
        .post(OAUTH_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to exchange code: {}", e))?;
    
    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed: {}", error_text));
    }
    
    let mut token: OAuthToken = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;
    
    // Calculate expiration time
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    token.expires_at = now + token.expires_in;
    
    // Store token
    {
        let mut oauth = state.lock().unwrap();
        oauth.token = Some(token.clone());
        oauth.pkce_verifier = None;
        oauth.state = None;
    }
    
    // Fetch user info
    fetch_user_info(&token.access_token, &state).await?;
    
    Ok(token)
}

async fn fetch_user_info(
    access_token: &str,
    state: &tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<OAuthUserInfo, String> {
    let client = reqwest::Client::new();
    
    let response = client
        .get(OAUTH_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch user info: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("User info request failed: {}", response.status()));
    }
    
    let user_info: OAuthUserInfo = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;
    
    // Store user info
    {
        let mut oauth = state.lock().unwrap();
        oauth.user_info = Some(user_info.clone());
    }
    
    Ok(user_info)
}

#[tauri::command]
pub async fn oauth_refresh_token(
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<OAuthToken, String> {
    let refresh_token = {
        let oauth = state.lock().unwrap();
        oauth.token.as_ref()
            .and_then(|t| t.refresh_token.clone())
            .ok_or_else(|| "No refresh token available".to_string())?
    };
    
    let client = reqwest::Client::new();
    
    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", &refresh_token);
    params.insert("client_id", CLIENT_ID);
    
    let response = client
        .post(OAUTH_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to refresh token: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("Token refresh failed: {}", response.status()));
    }
    
    let mut token: OAuthToken = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;
    
    // Calculate expiration time
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    token.expires_at = now + token.expires_in;
    
    // Store new token
    {
        let mut oauth = state.lock().unwrap();
        oauth.token = Some(token.clone());
    }
    
    Ok(token)
}

#[tauri::command]
pub async fn oauth_logout(
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<(), String> {
    let mut oauth = state.lock().unwrap();
    oauth.token = None;
    oauth.user_info = None;
    Ok(())
}

#[tauri::command]
pub async fn oauth_get_status(
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<(bool, Option<OAuthUserInfo>), String> {
    let oauth = state.lock().unwrap();
    Ok((oauth.is_authenticated(), oauth.user_info.clone()))
}

#[tauri::command]
pub async fn oauth_get_token(
    state: tauri::State<'_, std::sync::Mutex<OAuthState>>,
) -> Result<Option<String>, String> {
    let oauth = state.lock().unwrap();
    Ok(oauth.token.as_ref().map(|t| t.access_token.clone()))
}

// Start local callback server
pub async fn start_callback_server(app_handle: tauri::AppHandle) -> Result<()> {
    use warp::Filter;
    
    let app_handle = std::sync::Arc::new(app_handle);
    
    let callback = warp::path("callback")
        .and(warp::query::<HashMap<String, String>>())
        .and(warp::any().map(move || app_handle.clone()))
        .and_then(handle_callback);
    
    let routes = callback
        .or(warp::any().map(|| warp::reply::html(CALLBACK_HTML.to_string())));
    
    warp::serve(routes)
        .run(([127, 0, 0, 1], 9420))
        .await;
    
    Ok(())
}

async fn handle_callback(
    params: HashMap<String, String>,
    app_handle: std::sync::Arc<tauri::AppHandle>,
) -> Result<warp::reply::Html<String>, warp::Rejection> {
    if let (Some(code), Some(state)) = (params.get("code"), params.get("state")) {
        // Emit event to the frontend
        app_handle.as_ref().emit("oauth-callback", serde_json::json!({
            "code": code,
            "state": state
        })).unwrap();
        
        Ok(warp::reply::html(SUCCESS_HTML.to_string()))
    } else if let Some(error) = params.get("error") {
        let error_desc = params.get("error_description").unwrap_or(error);
        
        app_handle.as_ref().emit("oauth-error", serde_json::json!({
            "error": error,
            "description": error_desc
        })).unwrap();
        
        let error_html = ERROR_HTML.replace("%s", error_desc);
        Ok(warp::reply::html(error_html))
    } else {
        Ok(warp::reply::html(ERROR_HTML.to_string()))
    }
}

const CALLBACK_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>WindowsForum Authentication</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }
        .container {
            text-align: center;
            padding: 2rem;
            background: white;
            border-radius: 10px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }
        .spinner {
            border: 3px solid #f3f3f3;
            border-top: 3px solid #667eea;
            border-radius: 50%;
            width: 40px;
            height: 40px;
            animation: spin 1s linear infinite;
            margin: 0 auto 1rem;
        }
        @keyframes spin {
            0% { transform: rotate(0deg); }
            100% { transform: rotate(360deg); }
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="spinner"></div>
        <h2>Authenticating...</h2>
        <p>Please wait while we complete your authentication.</p>
    </div>
</body>
</html>
"#;

const SUCCESS_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Authentication Successful</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }
        .container {
            text-align: center;
            padding: 2rem;
            background: white;
            border-radius: 10px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }
        .success {
            color: #10b981;
            font-size: 3rem;
            margin-bottom: 1rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="success">✓</div>
        <h2>Authentication Successful!</h2>
        <p>You can now close this window and return to the application.</p>
    </div>
    <script>
        setTimeout(() => window.close(), 3000);
    </script>
</body>
</html>
"#;

const ERROR_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Authentication Failed</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }
        .container {
            text-align: center;
            padding: 2rem;
            background: white;
            border-radius: 10px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }
        .error {
            color: #ef4444;
            font-size: 3rem;
            margin-bottom: 1rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="error">✗</div>
        <h2>Authentication Failed</h2>
        <p>%s</p>
        <p>Please close this window and try again.</p>
    </div>
</body>
</html>
"#;