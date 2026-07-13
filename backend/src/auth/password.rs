use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::http::StatusCode;

use crate::router::error::{ApiError, err};

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("password must be at least {MIN_PASSWORD_LEN} characters"),
        ));
    }
    Ok(())
}

pub fn hash_password(plain: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| {
            tracing::error!("password hashing failed: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        })
}

pub fn verify_password(plain: &str, hash: &str) -> Result<bool, ApiError> {
    let parsed = PasswordHash::new(hash).map_err(|e| {
        tracing::error!("invalid password hash: {e}");
        err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    })?;
    Ok(Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok())
}
