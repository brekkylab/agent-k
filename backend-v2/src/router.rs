use std::{convert::Infallible, sync::Arc};

use aide::axum::{
    ApiRouter,
    routing::{delete, post},
};
use ailoy::{
    agent::{Agent, AgentBuilder, AgentCard},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SandboxConfig},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::Utc;
use futures_util::StreamExt;
use speedwagon::SpeedwagonSpec;
use uuid::Uuid;

use crate::{
    auth::{AuthUser, admin_required, auth_required, hash_password, validate_password, verify_password},
    error::{ApiResult, AppError},
    model::{
        CreateSessionRequest, SendMessageRequest, SendMessageResponse, SessionResponse,
        user::{
            AdminCreateUserRequest, AdminUpdateUserRequest, LoginRequest, LoginResponse,
            SignupRequest, UpdateMeRequest, UserListQuery, UserListResponse, UserResponse,
        },
    },
    repository::{NewUser, RepositoryError, UpdateUser},
    state::AppState,
};

const DEFAULT_MODEL: &str = "openai/gpt-5.4-mini";

fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    // Public auth endpoints (documented in OpenAPI)
    let public_routes = ApiRouter::new()
        .api_route("/auth/signup", post(signup))
        .api_route("/auth/login", post(login));

    // /me — requires JWT auth
    let me_routes = ApiRouter::new()
        .route("/me", axum::routing::get(get_me).patch(update_me))
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    // /admin — requires JWT auth + admin role
    let admin_routes = ApiRouter::new()
        .route(
            "/admin/users",
            axum::routing::get(list_users).post(create_user_admin),
        )
        .route(
            "/admin/users/{id}",
            axum::routing::get(get_user_admin)
                .patch(update_user_admin)
                .delete(delete_user_admin),
        )
        .layer(middleware::from_fn(admin_required))
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    // Existing session/message endpoints (unauthenticated for now)
    let session_routes = ApiRouter::new()
        .api_route("/sessions", post(create_session))
        .api_route("/sessions/{id}", delete(delete_session))
        .api_route("/sessions/{id}/messages", post(send_message))
        .route(
            "/sessions/{id}/messages/stream",
            axum::routing::post(send_message_stream),
        )
        .route(
            "/sessions/{id}/messages",
            axum::routing::get(get_message_history).delete(clear_message_history),
        );

    ApiRouter::new()
        .merge(public_routes)
        .merge(me_routes)
        .merge(admin_routes)
        .merge(session_routes)
        .with_state(state)
}

// ── Auth handlers ─────────────────────────────────────────────────────────────

async fn signup(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SignupRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let id = Uuid::new_v4();

    let user = state
        .repository
        .create_user(NewUser {
            id,
            username: payload.username,
            password_hash,
            role: crate::auth::Role::User,
            display_name: payload.display_name,
            is_active: true,
        })
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("username already taken"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(%id, username = %user.username, "user signed up");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let user = state
        .repository
        .get_user_by_username(&payload.username)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::unauthorized("invalid username or password"))?;

    if !user.is_active {
        return Err(AppError::forbidden("account is deactivated"));
    }

    if !verify_password(&payload.password, &user.password_hash)? {
        return Err(AppError::unauthorized("invalid username or password"));
    }

    let access_token = state
        .jwt
        .encode(user.id, user.username.clone(), user.role.clone())?;

    tracing::info!(id = %user.id, username = %user.username, "user logged in");

    Ok(Json(LoginResponse {
        token_type: "Bearer".to_string(),
        expires_in: state.jwt.expiry_secs,
        user: UserResponse::from(user),
        access_token,
    }))
}

// ── /me handlers ─────────────────────────────────────────────────────────────

async fn get_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> ApiResult<Json<UserResponse>> {
    let user = state
        .repository
        .get_user_by_id(auth.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(user)))
}

async fn update_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<UpdateMeRequest>,
) -> ApiResult<Json<UserResponse>> {
    let new_password_hash = if let Some(ref new_password) = payload.password {
        validate_password(new_password)?;

        let current_password = payload.current_password.as_deref().ok_or_else(|| {
            AppError::bad_request("current_password is required to change password")
        })?;

        let user = state
            .repository
            .get_user_by_id(auth.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
            .ok_or_else(|| AppError::not_found("user not found"))?;

        if !verify_password(current_password, &user.password_hash)? {
            return Err(AppError::unauthorized("current password is incorrect"));
        }

        Some(hash_password(new_password)?)
    } else {
        None
    };

    let updated = state
        .repository
        .update_user(
            auth.id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: None,
                is_active: None,
            },
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(updated)))
}

// ── Admin handlers ────────────────────────────────────────────────────────────

async fn list_users(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Query(q): Query<UserListQuery>,
) -> ApiResult<Json<UserListResponse>> {
    let page = q.page.unwrap_or(1);
    let size = q.size.unwrap_or(20);

    let (users, total) = state
        .repository
        .list_users(page, size)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(UserListResponse {
        items: users.into_iter().map(UserResponse::from).collect(),
        total,
    }))
}

async fn create_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Json(payload): Json<AdminCreateUserRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let id = Uuid::new_v4();
    let role = payload.role.unwrap_or(crate::auth::Role::User);
    let is_active = payload.is_active.unwrap_or(true);

    let user = state
        .repository
        .create_user(NewUser {
            id,
            username: payload.username,
            password_hash,
            role,
            display_name: payload.display_name,
            is_active,
        })
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("username already taken"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(%id, username = %user.username, "admin created user");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

async fn get_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UserResponse>> {
    let user = state
        .repository
        .get_user_by_id(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(user)))
}

async fn update_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AdminUpdateUserRequest>,
) -> ApiResult<Json<UserResponse>> {
    let new_password_hash = payload
        .password
        .as_deref()
        .map(|p| {
            validate_password(p)?;
            hash_password(p)
        })
        .transpose()?;

    let updated = state
        .repository
        .update_user(
            id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: payload.role,
                is_active: payload.is_active,
            },
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(updated)))
}

async fn delete_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if auth.id == id {
        return Err(AppError::bad_request("cannot delete your own account"));
    }

    let deleted = state
        .repository
        .delete_user(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if !deleted {
        return Err(AppError::not_found("user not found"));
    }

    tracing::info!(target_user_id = %id, by = %auth.id, "admin deleted user");

    Ok(StatusCode::NO_CONTENT)
}

// ── Session/message handlers (unchanged) ──────────────────────────────────────

async fn build_agent(sandbox: Sandbox) -> Result<Agent, String> {
    let sw_card = AgentCard {
        name: "speedwagon".into(),
        description: "Search the knowledge base for answers. \
            This tool has access to uploaded documents that may contain \
            information the model doesn't have. \
            Use it for any question that could be answered from the knowledge base."
            .into(),
        skills: vec![],
    };
    let sw_spec = SpeedwagonSpec::new().card(sw_card.clone()).into_spec();

    AgentBuilder::new(DEFAULT_MODEL)
        .instruction(concat!(
            "You are a versatile assistant with access to code execution tools ",
            "(bash, python), web search, and a knowledge base (speedwagon). ",
            "You MUST use the speedwagon tool to search the document corpus ",
            "before answering ANY factual question — even if you think you already know the answer. ",
            "The corpus contains authoritative information that may differ from your training data. ",
            "Use bash and python tools for computation, data analysis, and code execution tasks. ",
            "Only skip tools for greetings or casual conversation.",
        ))
        .tool("bash")
        .tool("python_repl")
        .tool("web_search")
        .runenv(sandbox)
        .subagent(sw_spec)
        .build()
        .await
        .map_err(|e| e.to_string())
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let id = Uuid::new_v4();
    let sandbox_name = sandbox_name_for(&id);

    let cfg = SandboxConfig {
        name: Some(sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    let now = Utc::now();
    state
        .repository
        .create_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    state.insert_agent(id, agent);

    tracing::info!(%id, sandbox = %sandbox_name, "session created");

    Ok((
        StatusCode::CREATED,
        Json(SessionResponse {
            id,
            created_at: now,
            updated_at: now,
        }),
    ))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }

    state
        .repository
        .delete_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let agent_arc = state.remove_agent(&id);

    if let Some(arc) = agent_arc {
        drop(arc.lock().await);
        drop(arc);
    }

    let sandbox_name = sandbox_name_for(&id);
    if let Err(e) = ailoy::runenv::remove_persisted(&sandbox_name).await {
        tracing::warn!(%id, "failed to remove persisted sandbox: {e}");
    }

    tracing::info!(%id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn resolve_agent(
    state: &Arc<AppState>,
    id: Uuid,
) -> ApiResult<Arc<tokio::sync::Mutex<Agent>>> {
    if let Some(arc) = state.get_agent(&id) {
        return Ok(arc);
    }

    let session_exists = state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();

    if !session_exists {
        return Err(AppError::not_found("session not found"));
    }

    let history = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let sandbox_name = sandbox_name_for(&id);
    let cfg = SandboxConfig {
        name: Some(sandbox_name),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let mut agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    agent.state.history = history;
    tracing::info!(%id, "agent lazy-created with history restored");

    if let Some(existing) = state.get_agent(&id) {
        return Ok(existing);
    }
    state.insert_agent(id, agent);
    Ok(state.get_agent(&id).unwrap())
}

async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<Message>>> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    let messages = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(messages))
}

async fn clear_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    state
        .repository
        .clear_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if let Some(arc) = state.get_agent(&id) {
        arc.lock().await.state.history.clear();
    }

    tracing::info!(%id, "message history cleared");
    Ok(StatusCode::NO_CONTENT)
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Json<SendMessageResponse>> {
    let agent_arc = resolve_agent(&state, id).await?;

    let prev_len = agent_arc.lock().await.get_history().len();

    let outputs = {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(payload.content)]);
        let mut stream = agent.run(msg);
        let mut outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = stream.next().await {
            outputs.push(item.map_err(|e| AppError::internal(e.to_string()))?);
        }
        outputs
    };

    let new_messages = {
        let agent = agent_arc.lock().await;
        agent.get_history()[prev_len..].to_vec()
    };
    state
        .repository
        .append_messages(id, &new_messages)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(outputs))
}

async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static>> {
    let agent_arc = resolve_agent(&state, id).await?;
    let repo = state.repository.clone();
    let prev_len = agent_arc.lock().await.get_history().len();
    let content = payload.content;

    let stream = async_stream::stream! {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(content)]);
        let mut run = agent.run(msg);

        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    let json = serde_json::to_string(&output)
                        .unwrap_or_else(|e| format!("{{\"error\":\"{e}}}", e = e));
                    yield Ok::<Event, Infallible>(
                        Event::default().event("message").data(json),
                    );
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                    return;
                }
            }
        }
        drop(run);

        let new_msgs = agent.get_history()[prev_len..].to_vec();
        if let Err(e) = repo.append_messages(id, &new_msgs).await {
            tracing::error!(%id, "failed to persist messages: {e}");
        }

        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
