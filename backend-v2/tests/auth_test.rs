#[path = "common/mod.rs"]
mod common;

use axum::http::StatusCode;

// ── Signup ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn signup_creates_user_with_role_user() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let body = common::signup(&app, "alice", "password123").await;

    assert_eq!(body["username"], "alice");
    assert_eq!(body["role"], "user");
    assert!(body["id"].is_string());
    assert!(body["is_active"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn signup_with_display_name() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let (status, body) = common::signup_status(&app, "bob", "password123", Some("Bob Smith")).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["display_name"], "Bob Smith");
}

#[tokio::test]
async fn signup_rejects_duplicate_username() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "carol", "password123").await;

    let (status, body) = common::signup_status(&app, "carol", "different123", None).await;

    assert_eq!(status, StatusCode::CONFLICT, "expected 409, got: {body}");
}

#[tokio::test]
async fn signup_rejects_short_password() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let (status, _body) = common::signup_status(&app, "dave", "short", None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── Login ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_returns_jwt_for_valid_credentials() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "eve", "password123").await;
    let (status, body) = common::login_status(&app, "eve", "password123").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert!(body["expires_in"].is_number());
    assert_eq!(body["user"]["username"], "eve");
}

#[tokio::test]
async fn login_rejects_wrong_password() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "frank", "password123").await;
    let (status, _body) = common::login_status(&app, "frank", "wrongpassword").await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_rejects_unknown_user() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let (status, _body) = common::login_status(&app, "nobody", "password123").await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ── /me ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn me_requires_authentication() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let req = Request::builder()
        .method("GET")
        .uri("/me")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_rejects_invalid_token() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let (status, _body) = common::authed(&app, "GET", "/me", "not-a-valid-jwt", None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_current_user_info() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "grace", "password123").await;
    let token = common::login(&app, "grace", "password123").await;

    let (status, body) = common::authed(&app, "GET", "/me", &token, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "grace");
    assert_eq!(body["role"], "user");
}

#[tokio::test]
async fn update_me_changes_display_name() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "heidi", "password123").await;
    let token = common::login(&app, "heidi", "password123").await;

    let payload = serde_json::json!({ "display_name": "Heidi Doe" });
    let (status, body) = common::authed(&app, "PATCH", "/me", &token, Some(payload)).await;

    assert_eq!(status, StatusCode::OK, "update me failed: {body}");
    assert_eq!(body["display_name"], "Heidi Doe");
}

#[tokio::test]
async fn update_me_password_requires_current_password() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "ivan", "password123").await;
    let token = common::login(&app, "ivan", "password123").await;

    let payload = serde_json::json!({ "password": "newpassword99" });
    let (status, _body) = common::authed(&app, "PATCH", "/me", &token, Some(payload)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_me_password_with_correct_current() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "judy", "oldpassword1").await;
    let token = common::login(&app, "judy", "oldpassword1").await;

    let payload = serde_json::json!({
        "password": "newpassword2",
        "current_password": "oldpassword1"
    });
    let (status, _body) = common::authed(&app, "PATCH", "/me", &token, Some(payload)).await;
    assert_eq!(status, StatusCode::OK);

    // Old password should no longer work
    let (old_status, _) = common::login_status(&app, "judy", "oldpassword1").await;
    assert_eq!(old_status, StatusCode::UNAUTHORIZED);

    // New password should work
    let (new_status, _) = common::login_status(&app, "judy", "newpassword2").await;
    assert_eq!(new_status, StatusCode::OK);
}

#[tokio::test]
async fn update_me_password_rejects_wrong_current() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "kevin", "password123").await;
    let token = common::login(&app, "kevin", "password123").await;

    let payload = serde_json::json!({
        "password": "newpassword99",
        "current_password": "wrongcurrent"
    });
    let (status, _body) = common::authed(&app, "PATCH", "/me", &token, Some(payload)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ── Admin: access control ────────────────────────────────────────────────────

#[tokio::test]
async fn admin_endpoints_reject_unauthenticated() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let req = Request::builder()
        .method("GET")
        .uri("/admin/users")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_endpoints_reject_non_admin() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "lena", "password123").await;
    let token = common::login(&app, "lena", "password123").await;

    let (status, _body) = common::authed(&app, "GET", "/admin/users", &token, None).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── Admin: user management ────────────────────────────────────────────────────

#[allow(dead_code)]
async fn setup_admin(_app: &axum::Router) -> (serde_json::Value, String) {
    // Create admin via repository directly for speed
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    // We create via the public API: signup then manually promote (not ideal but works for tests)
    // Instead, create via repo. But we can't easily get the repo here.
    // Use the signup + direct DB approach... actually let's just use the create-admin via repo.
    // For simplicity, sign up and then create admin via admin API (chicken-and-egg problem).
    // Workaround: sign up a "bootstrap" admin via repository pre-seeded state.
    // Actually, let's just create the admin using signup but override role in DB.
    // Better: make a helper that creates an admin via repo directly.

    // The cleanest way is to expose repo from make_app, but that changes the API.
    // For now, we'll build the app state manually so we can access the repo.
    let repo = common::make_repo().await;
    let jwt = common::test_jwt_config();

    let password_hash = auth::hash_password("adminpass1").unwrap();
    let admin_user = repo
        .create_user(NewUser {
            id: uuid::Uuid::new_v4(),
            username: "admin".to_string(),
            password_hash,
            role: auth::Role::Admin,
            display_name: Some("Admin".to_string()),
            is_active: true,
        })
        .await
        .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        jwt.clone(),
    ));
    let app = common::make_app_with_state(state);

    let token = common::login(&app, "admin", "adminpass1").await;

    (
        serde_json::json!({ "id": admin_user.id.to_string(), "username": "admin" }),
        token,
    )
}

#[tokio::test]
async fn admin_can_create_and_list_users() {
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;
    let password_hash = auth::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin".to_string(),
        password_hash,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    let admin_token = common::login(&app, "admin", "adminpass1").await;

    // Create a user via admin API
    let payload = serde_json::json!({
        "username": "newuser",
        "password": "newpassword1",
        "display_name": "New User"
    });
    let (status, created) =
        common::authed(&app, "POST", "/admin/users", &admin_token, Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED, "create user failed: {created}");
    assert_eq!(created["username"], "newuser");
    assert_eq!(created["role"], "user");

    // List users
    let (status, list) = common::authed(&app, "GET", "/admin/users", &admin_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["total"].as_i64().unwrap(), 2); // admin + newuser
}

#[tokio::test]
async fn admin_can_update_user_role() {
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;
    let password_hash = auth::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin".to_string(),
        password_hash,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    let admin_token = common::login(&app, "admin", "adminpass1").await;

    // Create regular user via signup
    let user_info = common::signup(&app, "regularuser", "password123").await;
    let user_id = user_info["id"].as_str().unwrap();

    // Promote to admin
    let payload = serde_json::json!({ "role": "admin" });
    let uri = format!("/admin/users/{user_id}");
    let (status, updated) = common::authed(&app, "PATCH", &uri, &admin_token, Some(payload)).await;
    assert_eq!(status, StatusCode::OK, "update failed: {updated}");
    assert_eq!(updated["role"], "admin");
}

#[tokio::test]
async fn admin_can_deactivate_user() {
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;
    let password_hash = auth::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin".to_string(),
        password_hash,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    let admin_token = common::login(&app, "admin", "adminpass1").await;

    common::signup(&app, "victim", "password123").await;

    // Get user id via list
    let (_, list) = common::authed(&app, "GET", "/admin/users", &admin_token, None).await;
    let victim = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "victim")
        .unwrap()
        .clone();
    let victim_id = victim["id"].as_str().unwrap();

    // Deactivate
    let payload = serde_json::json!({ "is_active": false });
    let uri = format!("/admin/users/{victim_id}");
    let (status, _) = common::authed(&app, "PATCH", &uri, &admin_token, Some(payload)).await;
    assert_eq!(status, StatusCode::OK);

    // Deactivated user can't login
    let (login_status, _) = common::login_status(&app, "victim", "password123").await;
    assert_eq!(login_status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_delete_user() {
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;
    let password_hash = auth::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin".to_string(),
        password_hash,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    let admin_token = common::login(&app, "admin", "adminpass1").await;

    let user_info = common::signup(&app, "todelete", "password123").await;
    let user_id = user_info["id"].as_str().unwrap();

    let uri = format!("/admin/users/{user_id}");
    let (status, _) = common::authed(&app, "DELETE", &uri, &admin_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify deleted
    let (get_status, _) = common::authed(&app, "GET", &uri, &admin_token, None).await;
    assert_eq!(get_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_cannot_delete_self() {
    use std::sync::Arc;

    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;
    let password_hash = auth::hash_password("adminpass1").unwrap();
    let admin_user = repo
        .create_user(NewUser {
            id: uuid::Uuid::new_v4(),
            username: "admin".to_string(),
            password_hash,
            role: auth::Role::Admin,
            display_name: None,
            is_active: true,
        })
        .await
        .unwrap();

    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo,
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    let admin_token = common::login(&app, "admin", "adminpass1").await;
    let admin_id = admin_user.id;

    let uri = format!("/admin/users/{admin_id}");
    let (status, body) = common::authed(&app, "DELETE", &uri, &admin_token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400, got: {body}");
}

// ── Bootstrap ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn count_admins_starts_at_zero_and_increments() {
    use agent_k_backend::{auth, repository::NewUser};

    let repo = common::make_repo().await;

    assert_eq!(repo.count_admins().await.unwrap(), 0);

    let password_hash = auth::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin".to_string(),
        password_hash,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    assert_eq!(repo.count_admins().await.unwrap(), 1);
}
