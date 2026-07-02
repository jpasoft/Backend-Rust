use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use tokio::net::TcpListener;
use bcrypt::{hash, verify, DEFAULT_COST};

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

// ==================== ESTRUCTURAS DE RESPUESTA ====================

#[derive(Serialize)]
struct ApiResponse {
    message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    redirect: String,
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    user_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

// ==================== LOGIN / REGISTER ====================

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct RegisterRequest {
    gender: String,
    username: String,
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    let result = sqlx::query("SELECT id, password, username FROM users WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&state.db)
        .await;

    match result {
        Ok(Some(row)) => {
            let id: i32 = row.get("id");
            let db_password: String = row.get("password");
            let username: String = row.get("username");
            if verify(&payload.password, &db_password).unwrap_or(false) {
                (StatusCode::OK, Json(ApiResponse {
                    message: "Login correcto".to_string(),
                    redirect: "/profile".to_string(),
                    user_id: Some(id),
                    username: Some(username),
                }))
            } else {
                (StatusCode::UNAUTHORIZED, Json(ApiResponse { message: "Credenciales incorrectas".to_string(), redirect: "".to_string(), user_id: None, username: None }))
            }
        }
        _ => (StatusCode::UNAUTHORIZED, Json(ApiResponse { message: "Credenciales incorrectas".to_string(), redirect: "".to_string(), user_id: None, username: None })),
    }
}

async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> impl IntoResponse {
    let hashed = match hash(&payload.password, DEFAULT_COST) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse { message: "Error interno".to_string(), redirect: "".to_string(), user_id: None, username: None })),
    };

    let result = sqlx::query("INSERT INTO users (gender, username, email, password) VALUES ($1, $2, $3, $4)")
        .bind(&payload.gender)
        .bind(&payload.username)
        .bind(&payload.email)
        .bind(&hashed)
        .execute(&state.db)
        .await;

    match result {
        Ok(_) => (StatusCode::OK, Json(ApiResponse { message: "Usuario registrado".to_string(), redirect: "".to_string(), user_id: None, username: None })),
        Err(e) => {
            eprintln!("DB ERROR REGISTER: {:?}", e);
            (StatusCode::BAD_REQUEST, Json(ApiResponse { message: "Error registro".to_string(), redirect: "".to_string(), user_id: None, username: None }))
        }
    }
}

// ==================== FAVORITOS ====================

#[derive(Deserialize)]
struct FavoriteRequest {
    #[serde(rename = "userId")]
    user_id: i32,
    #[serde(rename = "gameId")]
    game_id: i32,
}

#[derive(Deserialize)]
struct GetFavQuery {
    #[serde(rename = "userId")]
    user_id: i32,
}

async fn add_favorite(State(state): State<AppState>, Json(payload): Json<FavoriteRequest>) -> impl IntoResponse {
    let result = sqlx::query("INSERT INTO favgames (user_id, game_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(payload.user_id)
        .bind(payload.game_id)
        .execute(&state.db)
        .await;

    if result.is_ok() { StatusCode::CREATED } else { StatusCode::INTERNAL_SERVER_ERROR }
}

async fn delete_favorite(State(state): State<AppState>, Json(payload): Json<FavoriteRequest>) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM favgames WHERE user_id = $1 AND game_id = $2")
        .bind(payload.user_id)
        .bind(payload.game_id)
        .execute(&state.db)
        .await;

    if result.is_ok() { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR }
}

async fn get_favorites(
    State(state): State<AppState>,
    Query(query): Query<GetFavQuery>
) -> impl IntoResponse {
    let result = sqlx::query("SELECT game_id FROM favgames WHERE user_id = $1")
        .bind(query.user_id)
        .fetch_all(&state.db)
        .await;

    match result {
        Ok(rows) => (StatusCode::OK, Json(rows.into_iter().map(|r| r.get::<i32, _>("game_id")).collect::<Vec<i32>>())),
        Err(e) => {
            eprintln!("Error al obtener favoritos: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(vec![]))
        },
    }
}

// ==================== USERS, ROOMS, MESSAGES, MINIGAMES ====================

#[derive(Serialize)]
struct User { id: i32, username: String, email: String }

async fn get_users(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query("SELECT id, username, email FROM users")
        .fetch_all(&state.db)
        .await
        .unwrap_or_else(|e| { eprintln!("DB error get_users: {:?}", e); vec![] });
    let users: Vec<User> = rows.into_iter().map(|r| User { id: r.get("id"), username: r.get("username"), email: r.get("email") }).collect();
    Json(users)
}

#[derive(Serialize)]
struct Room { id: i32, name: String, description: Option<String>, icon_url: Option<String> }

async fn get_rooms(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query("SELECT id, name, description, icon_url FROM rooms ORDER BY id DESC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_else(|e| { eprintln!("DB error get_rooms: {:?}", e); vec![] });
    let rooms: Vec<Room> = rows.into_iter().map(|r| Room { id: r.get("id"), name: r.get("name"), description: r.get("description"), icon_url: r.get("icon_url") }).collect();
    Json(rooms)
}

#[derive(Serialize)]
struct Message { username: String, content: String, nick_color: Option<String> }

#[derive(Deserialize)]
struct CreateMessage { username: String, content: String, nick_color: Option<String> }

async fn get_messages(State(state): State<AppState>, Path(room_id): Path<i32>) -> impl IntoResponse {
    let rows = sqlx::query("SELECT username, content, nick_color FROM messages WHERE room_id = $1 ORDER BY id ASC")
        .bind(room_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_else(|e| { eprintln!("DB error get_messages: {:?}", e); vec![] });
    let msgs: Vec<Message> = rows.into_iter().map(|r| Message {
        username: r.get("username"),
        content: r.get("content"),
        nick_color: r.get("nick_color"),
    }).collect();
    Json(msgs)
}

async fn create_message(State(state): State<AppState>, Path(room_id): Path<i32>, Json(payload): Json<CreateMessage>) -> impl IntoResponse {
    let res = sqlx::query("INSERT INTO messages (room_id, username, content, nick_color) VALUES ($1, $2, $3, $4)")
        .bind(room_id)
        .bind(&payload.username)
        .bind(&payload.content)
        .bind(&payload.nick_color)
        .execute(&state.db)
        .await;
    if res.is_ok() { StatusCode::CREATED } else { StatusCode::INTERNAL_SERVER_ERROR }
}

#[derive(Serialize)]
struct MiniGame { id: i32, title: String, description: String, icon_url: Option<String>, game_url: Option<String>, game_category: Option<Vec<String>>, render_type: Option<String>, screenshot_1: Option<String>, screenshot_2: Option<String>, screenshot_3: Option<String>, screenshot_4: Option<String>, screenshot_5: Option<String>, screenshot_6: Option<String>, screenshot_7: Option<String> }

async fn get_minigames(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query("SELECT id, title, description, icon_url, game_url, game_category, render_type, screenshot_1, screenshot_2, screenshot_3, screenshot_4, screenshot_5, screenshot_6, screenshot_7 FROM minigames ORDER BY id ASC")
        .fetch_all(&state.db)
        .await
        .unwrap_or_else(|e| { eprintln!("DB error get_minigames: {:?}", e); vec![] });
    let games: Vec<MiniGame> = rows.into_iter().map(|r| MiniGame { id: r.get("id"), title: r.get("title"), description: r.get("description"), icon_url: r.get("icon_url"), game_url: r.get("game_url"), game_category: r.get("game_category"), render_type: r.get("render_type"), screenshot_1: r.get("screenshot_1"), screenshot_2: r.get("screenshot_2"), screenshot_3: r.get("screenshot_3"), screenshot_4: r.get("screenshot_4"), screenshot_5: r.get("screenshot_5"), screenshot_6: r.get("screenshot_6"), screenshot_7: r.get("screenshot_7") }).collect();
    Json(games)
}

// ==================== MAIN ====================

#[tokio::main]
async fn main() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://user_db:mypassdb@192.168.18.41:5432/pg_db".to_string());
    let db = PgPool::connect(&db_url).await.expect("Error DB");

    let app = Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/users", get(get_users))
        .route("/rooms", get(get_rooms))
        .route("/chatrooms/:id/messages", get(get_messages).post(create_message))
        .route("/minigames", get(get_minigames))
        .route("/favorites", get(get_favorites).post(add_favorite).delete(delete_favorite))
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { db })
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let listener = TcpListener::bind("192.168.18.41:8080").await.unwrap();
    println!("Servidor en http://192.168.18.41:8080");
    axum::serve(listener, app).await.unwrap();
}