use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::role::Role,
    error::{ApiError, AppError},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: Uuid,
    pub username: String,
    pub role: Role,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone)]
pub struct JwtConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    pub expiry_secs: u64,
}

impl JwtConfig {
    pub fn new(secret: &str, expiry_secs: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            expiry_secs,
        }
    }

    pub fn encode(&self, user_id: Uuid, username: String, role: Role) -> Result<String, ApiError> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: user_id,
            username,
            role,
            iat: now,
            exp: now + self.expiry_secs as i64,
        };
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AppError::internal(format!("JWT encode error: {e}")))
    }

    pub fn decode(&self, token: &str) -> Result<Claims, ApiError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        decode::<Claims>(token, &self.decoding_key, &validation)
            .map(|d| d.claims)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    AppError::unauthorized("token has expired")
                }
                _ => AppError::unauthorized("invalid token"),
            })
    }
}
