use uuid::Uuid;

use crate::{
    auth::{Role, hash_password},
    state::{NewUser, UsersState},
};

pub async fn bootstrap_admin_if_needed(users: &UsersState) {
    let count = match users.count_admins().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to count admin users: {e}");
            return;
        }
    };

    if count > 0 {
        return;
    }

    let username = std::env::var("AGENT_K_ADMIN_USERNAME");
    let password = std::env::var("AGENT_K_ADMIN_PASSWORD");

    match (username, password) {
        (Ok(u), Ok(p)) => {
            let password_hash = match hash_password(&p) {
                Ok(h) => h,
                Err(_) => {
                    tracing::error!("failed to hash bootstrap admin password");
                    return;
                }
            };

            match users
                .create(NewUser {
                    id: Uuid::new_v4(),
                    username: u.clone(),
                    password_hash,
                    role: Role::Admin,
                    display_name: None,
                    is_active: true,
                    preferred_language: "en".to_string(),
                })
                .await
            {
                Ok(user) => {
                    tracing::info!(
                        id = %user.id, username = %u,
                        "bootstrap admin user created from env"
                    );
                }
                Err(e) => {
                    tracing::error!("failed to create bootstrap admin: {e}");
                }
            }
        }
        _ => {
            tracing::warn!(
                "no admin user exists — set AGENT_K_ADMIN_USERNAME/AGENT_K_ADMIN_PASSWORD"
            );
        }
    }
}
