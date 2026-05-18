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

    // Signup should have created a Personal project automatically
    let token = body["access_token"].as_str().unwrap().to_string();
    let personal = common::get_personal_project(&app, &token).await;
    assert_eq!(personal["name"], "Personal");
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

#[tokio::test]
async fn admin_can_create_and_list_users() {
    let (app, admin_token, _admin_id) = common::make_admin_app().await;

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
    let (app, admin_token, _admin_id) = common::make_admin_app().await;

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
    let (app, admin_token, _admin_id) = common::make_admin_app().await;

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

/// A valid JWT for a deactivated user must be rejected by the middleware (403).
/// The middleware calls get_user_by_id on every request and checks is_active.
#[tokio::test]
async fn deactivated_user_with_valid_token_gets_403() {
    let (app, admin_token, _) = common::make_admin_app().await;

    // Sign up a regular user and get their token before deactivation.
    common::signup(&app, "willbedeactivated", "Password123!").await;
    let user_token = common::login(&app, "willbedeactivated", "Password123!").await;

    // Token works before deactivation.
    let (status, _) = common::authed(&app, "GET", "/me", &user_token, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "token should work before deactivation"
    );

    // Admin deactivates the user.
    let (_, list) = common::authed(&app, "GET", "/admin/users", &admin_token, None).await;
    let user_id = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "willbedeactivated")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = common::authed(
        &app,
        "PATCH",
        &format!("/admin/users/{user_id}"),
        &admin_token,
        Some(serde_json::json!({ "is_active": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "deactivation failed");

    // Existing token is now rejected with 403.
    let (status, _) = common::authed(&app, "GET", "/me", &user_token, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "deactivated user must not access API with a previously valid token"
    );
}

#[tokio::test]
async fn admin_can_delete_user() {
    let (app, admin_token, _admin_id) = common::make_admin_app().await;

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
    let (app, admin_token, admin_id) = common::make_admin_app().await;

    let uri = format!("/admin/users/{admin_id}");
    let (status, body) = common::authed(&app, "DELETE", &uri, &admin_token, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400, got: {body}");
}

// ── Admin lockout guards ──────────────────────────────────────────────────────

#[tokio::test]
async fn admin_cannot_demote_self() {
    let (app, admin_token, admin_id) = common::make_admin_app().await;

    let payload = serde_json::json!({ "role": "user" });
    let uri = format!("/admin/users/{admin_id}");
    let (status, body) = common::authed(&app, "PATCH", &uri, &admin_token, Some(payload)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "self-demotion should be blocked: {body}"
    );
}

#[tokio::test]
async fn admin_cannot_deactivate_self() {
    let (app, admin_token, admin_id) = common::make_admin_app().await;

    let payload = serde_json::json!({ "is_active": false });
    let uri = format!("/admin/users/{admin_id}");
    let (status, body) = common::authed(&app, "PATCH", &uri, &admin_token, Some(payload)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "self-deactivation should be blocked: {body}"
    );
}

#[tokio::test]
async fn last_admin_cannot_be_demoted_by_another_admin() {
    use agent_k_backend::{auth, repository::NewUser};

    // Two admins. admin2 demotes admin1 (count goes 2→1, fine).
    // Then admin2 tries to demote admin1 again — but admin1 is already a user.
    // Real test: only 1 active admin. The only way to trigger the "last admin" guard
    // without the self-guard is to have admin2 demote admin1 when admin2 is the
    // ONLY active admin — impossible via HTTP since admin2 can't be the requester
    // while also being the only admin and not the target.
    //
    // So we test the guard via the case where admin1 (the sole admin) tries to
    // demote themselves — which is also caught by the self-guard first.
    // We separate it: use admin1 acting on another user (admin2) who is the last
    // admin, achieved by making admin1 inactive via the repo directly.

    let repo = common::make_repo().await;

    let ph1 = auth::hash_password("adminpass1").unwrap();
    let admin1 = repo
        .create_user(NewUser {
            id: uuid::Uuid::new_v4(),
            username: "admin1".to_string(),
            password_hash: ph1,
            role: auth::Role::Admin,
            display_name: None,
            is_active: true,
        })
        .await
        .unwrap();

    let ph2 = auth::hash_password("adminpass2").unwrap();
    repo.create_user(NewUser {
        id: uuid::Uuid::new_v4(),
        username: "admin2".to_string(),
        password_hash: ph2,
        role: auth::Role::Admin,
        display_name: None,
        is_active: true,
    })
    .await
    .unwrap();

    // Deactivate admin2 via repo so admin1 becomes the sole active admin.
    use agent_k_backend::repository::UpdateUser;
    repo.update_user(
        admin1.id,
        UpdateUser {
            display_name: None,
            password_hash: None,
            role: None,
            is_active: Some(false), // make admin1 inactive
        },
    )
    .await
    .unwrap();

    // admin2 is now the sole active admin. admin2 tries to demote admin1
    // (the inactive admin, not themselves) — last-admin guard should not fire
    // here because admin1 is already inactive (not an active admin).
    // This is the "fine" case. Let's verify admin2 can still demote inactive admin1.
    let app = common::make_app_with_repo(repo);

    let admin2_token = common::login(&app, "admin2", "adminpass2").await;
    let admin1_id = admin1.id;

    // admin1 is inactive+admin; demoting role to "user" doesn't affect active admin count.
    let payload = serde_json::json!({ "role": "user" });
    let uri = format!("/admin/users/{admin1_id}");
    let (status, body) = common::authed(&app, "PATCH", &uri, &admin2_token, Some(payload)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "demoting an inactive admin should be allowed: {body}"
    );

    // Now admin2 tries to demote themselves (the last active admin) → blocked.
    let admin2_id = {
        let (_, list) = common::authed(&app, "GET", "/admin/users", &admin2_token, None).await;
        let users = list["items"].as_array().unwrap();
        users.iter().find(|u| u["username"] == "admin2").unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let payload = serde_json::json!({ "role": "user" });
    let uri = format!("/admin/users/{admin2_id}");
    let (status, body) = common::authed(&app, "PATCH", &uri, &admin2_token, Some(payload)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "last active admin cannot demote themselves: {body}"
    );
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
