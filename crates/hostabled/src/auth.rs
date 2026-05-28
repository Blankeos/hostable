use anyhow::{Context, Result, bail};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "hostable_session";
pub const OPERATOR_TOKEN_HEADER: &str = "x-hostable-operator-token";

#[derive(Clone)]
pub struct AuthStore {
    password_hash_path: PathBuf,
    operator_token_path: PathBuf,
    sessions: Arc<Mutex<HashSet<String>>>,
}

impl AuthStore {
    pub fn new(password_hash_path: PathBuf, operator_token_path: PathBuf) -> Self {
        Self {
            password_hash_path,
            operator_token_path,
            sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn setup_required(&self) -> bool {
        !self.password_hash_path.exists()
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        validate_password(password)?;
        if let Some(parent) = self.password_hash_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))?
            .to_string();
        fs::write(&self.password_hash_path, hash)?;
        Ok(())
    }

    pub fn verify_password(&self, password: &str) -> Result<bool> {
        if self.setup_required() {
            return Ok(false);
        }

        let hash = fs::read_to_string(&self.password_hash_path)
            .with_context(|| format!("failed to read {}", self.password_hash_path.display()))?;
        let parsed = PasswordHash::new(hash.trim())
            .map_err(|error| anyhow::anyhow!("invalid password hash: {error}"))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    pub fn create_session(&self) -> String {
        let token = Uuid::new_v4().to_string();
        self.sessions
            .lock()
            .expect("auth session mutex poisoned")
            .insert(token.clone());
        token
    }

    pub fn remove_session(&self, token: &str) {
        self.sessions
            .lock()
            .expect("auth session mutex poisoned")
            .remove(token);
    }

    pub fn valid_session(&self, token: &str) -> bool {
        self.sessions
            .lock()
            .expect("auth session mutex poisoned")
            .contains(token)
    }

    pub fn authenticated_headers(&self, headers: &HeaderMap) -> bool {
        session_from_headers(headers)
            .map(|token| self.valid_session(&token))
            .unwrap_or(false)
            || operator_token_from_headers(headers)
                .map(|token| self.valid_operator_token(&token))
                .unwrap_or(false)
    }

    fn valid_operator_token(&self, token: &str) -> bool {
        let Ok(expected) = fs::read_to_string(&self.operator_token_path) else {
            return false;
        };
        let expected = expected.trim();
        !expected.is_empty() && expected == token
    }
}

pub fn session_cookie(token: &str) -> String {
    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000")
}

pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

pub fn session_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })
}

pub fn operator_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(OPERATOR_TOKEN_HEADER)?
        .to_str()
        .ok()
        .map(str::to_string)
}

pub async fn require_auth(State(auth): State<AuthStore>, request: Request, next: Next) -> Response {
    if auth.authenticated_headers(request.headers()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response()
    }
}

fn validate_password(password: &str) -> Result<()> {
    if password.len() < 10 {
        bail!("admin password must be at least 10 characters");
    }
    Ok(())
}
