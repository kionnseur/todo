use std::{env, str::FromStr, sync::Arc, time::Duration};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post}, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation};
use rand::{rngs::SysRng, TryRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, FromRow, SqlitePool};
use tracing::{error, info};
use url::Url;

#[derive(Clone)]
struct AppState { db: SqlitePool, http: reqwest::Client, oidc: Discovery, issuer: String, client_id: String, client_secret: String, public_url: String }
#[derive(Debug, Clone, Deserialize)]
struct Discovery { issuer: String, authorization_endpoint: String, token_endpoint: String, jwks_uri: String }
#[derive(Debug, Deserialize)]
struct TokenResponse { id_token: String }
#[derive(Debug, Deserialize)]
struct Claims { sub: String, nonce: String, #[serde(default)] name: Option<String>, #[serde(default)] preferred_username: Option<String>, #[serde(default)] email: Option<String>, exp: usize }
#[derive(Debug, FromRow)]
struct AuthFlow { verifier: String, nonce: String, browser_hash: String }
#[derive(Debug, FromRow)]
struct User { sub: String, name: String }
#[derive(Debug, Serialize, FromRow)]
struct Task { id: String, parent_id: Option<String>, title: String, description: String, sort_order: i64, completed_at: Option<i64>, created_at: i64 }
#[derive(Debug, Deserialize)]
struct TaskInput { title: String, #[serde(default)] description: String, #[serde(default)] parent_id: Option<String> }
#[derive(Debug, Deserialize)]
struct CheckInput { done: bool }
#[derive(Debug, Deserialize)]
struct MoveInput { direction: String }
#[derive(Debug, Deserialize)]
struct CallbackQuery { code: Option<String>, state: Option<String>, error: Option<String> }

type ApiResult<T> = std::result::Result<T, (StatusCode, Json<serde_json::Value>)>;
fn api_error(status: StatusCode, reason: &str) -> (StatusCode, Json<serde_json::Value>) { (status, Json(serde_json::json!({"error":reason}))) }
fn server_error(err: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) { error!("Database/handler error: {err}"); api_error(StatusCode::INTERNAL_SERVER_ERROR, "Erreur interne") }
fn random_url_token() -> String { let mut b=[0u8;32];SysRng.try_fill_bytes(&mut b).expect("OS random source unavailable");URL_SAFE_NO_PAD.encode(b) }
fn digest(v: &str) -> String { Sha256::digest(v.as_bytes()).iter().map(|b| format!("{b:02x}")).collect() }
fn cookie(headers: &HeaderMap, key: &str) -> Option<String> {
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|p| { let (k,v)=p.trim().split_once('=')?; (k==key).then(||v.to_string()) })
}
fn set_cookie(name: &str, value: &str, age: i64) -> String {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={age}")
}
fn html_escape(input: &str) -> String { input.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&#39;") }
fn shell(content: &str, title: &str) -> Html<String> {
    Html(format!("<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"theme-color\" content=\"#f6f7f9\"><title>{}</title><link rel=\"stylesheet\" href=\"/static/style.css\"></head><body>{}<script src=\"/static/app.js\" defer></script></body></html>",html_escape(title),content))
}
fn add_security_headers(mut res: Response) -> Response {
    let h=res.headers_mut();
    h.insert("x-content-type-options",HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy",HeaderValue::from_static("same-origin"));
    h.insert("x-frame-options",HeaderValue::from_static("DENY"));
    h.insert("content-security-policy",HeaderValue::from_static("default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"));
    h.insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));
    res
}
fn guard_write(h: &HeaderMap, s: &AppState) -> ApiResult<()> {
    if h.get("x-todo-request").and_then(|v|v.to_str().ok())!=Some("1") { return Err(api_error(StatusCode::FORBIDDEN,"Requête non autorisée")); }
    if let Some(origin)=h.get(header::ORIGIN) { if origin.to_str().unwrap_or("")!=s.public_url { return Err(api_error(StatusCode::FORBIDDEN,"Origine non autorisée")); } }
    Ok(())
}
async fn current_user(s: &AppState, h: &HeaderMap) -> Option<User> {
    let sid=cookie(h,"todo_session")?;
    if sid.len()!=43 { return None; }
    sqlx::query_as::<_,User>("SELECT u.sub,u.name FROM sessions x JOIN users u ON u.sub=x.user_sub WHERE x.token_hash=? AND x.expires_at>?")
        .bind(digest(&sid)).bind(Utc::now().timestamp()).fetch_optional(&s.db).await.ok().flatten()
}
async fn require_user(s: &AppState, h: &HeaderMap) -> ApiResult<User> {
    current_user(s,h).await.ok_or_else(||api_error(StatusCode::UNAUTHORIZED,"Connexion requise"))
}
async fn index(State(s):State<Arc<AppState>>,headers:HeaderMap)->Response {
    let Some(user)=current_user(&s,&headers).await else {
        return add_security_headers(shell("<main class=\"wrap\"><section class=\"panel login\"><div class=\"logo\" style=\"margin:auto\">✓</div><h1>Mes tâches</h1><p>Une liste simple, privée, et toujours à portée de main.</p><a class=\"primary\" href=\"/auth/login\">Se connecter avec Pocket ID</a></section></main>","Todo · Connexion").into_response());
    };
    let html=format!(r#"<main class="wrap"><header class="topbar"><div class="brand"><div class="logo">✓</div><div><div class="title">Todo</div><div class="sub">L'essentiel, simplement.</div></div></div><div class="person"><span>{}</span><a href="/auth/logout" class="link">Déconnexion</a></div></header><section class="panel"><div class="heading"><div><h1>Mes tâches</h1><div class="count" id="count">Chargement…</div></div><button class="new" id="new-task">+ Nouvelle tâche</button></div><div id="tree" class="tree"><div class="empty">Chargement…</div></div></section><p class="footer">Les tâches cochées sont supprimées après 24 heures · Espace personnel</p></main><div class="overlay hidden" id="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><form class="modal" id="task-form"><h2 id="modal-title">Nouvelle tâche</h2><input type="hidden" name="id"><input type="hidden" name="parent_id"><label class="field" for="task-title">Titre *</label><input class="fieldinput" id="task-title" name="title" maxlength="160" required autofocus placeholder="Que faut-il faire ?"><label class="field" for="task-description">Description (facultative)</label><textarea class="fieldinput fieldtextarea" id="task-description" name="description" maxlength="10000" placeholder="Quelques détails utiles…"></textarea><div class="modal-actions"><button class="muted" id="cancel" type="button">Annuler</button><button class="primary" type="submit">Enregistrer</button></div></form></div>"#,html_escape(&user.name));
    add_security_headers(shell(&html,"Todo · Mes tâches").into_response())
}
async fn style()->Response { let mut r=( [(header::CONTENT_TYPE,"text/css; charset=utf-8")],include_str!("../static/style.css") ).into_response();r.headers_mut().insert("x-content-type-options",HeaderValue::from_static("nosniff"));r }
async fn script()->Response { let mut r=( [(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../static/app.js") ).into_response();r.headers_mut().insert("x-content-type-options",HeaderValue::from_static("nosniff"));r }
async fn health()->impl IntoResponse {(StatusCode::OK,"ok")}
async fn auth_login(State(s):State<Arc<AppState>>)->Response {
    let state=random_url_token();let nonce=random_url_token();let verifier=random_url_token();let browser=random_url_token();
    let challenge=URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let saved=sqlx::query("INSERT INTO auth_flows (state_hash,browser_hash,verifier,nonce,expires_at) VALUES (?,?,?,?,?)")
       .bind(digest(&state)).bind(digest(&browser)).bind(&verifier).bind(&nonce).bind(Utc::now().timestamp()+600).execute(&s.db).await;
    if let Err(e)=saved {error!("OIDC flow save: {e}");return StatusCode::INTERNAL_SERVER_ERROR.into_response();}
    let mut u=match Url::parse(&s.oidc.authorization_endpoint) {Ok(u)=>u,Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response()};
    u.query_pairs_mut().append_pair("response_type","code").append_pair("client_id",&s.client_id)
        .append_pair("redirect_uri",&format!("{}/auth/callback",s.public_url)).append_pair("scope","openid profile email")
        .append_pair("state",&state).append_pair("nonce",&nonce).append_pair("code_challenge",&challenge).append_pair("code_challenge_method","S256");
    let mut resp=Redirect::to(u.as_str()).into_response();
    resp.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_oidc",&browser,600)).unwrap());
    add_security_headers(resp)
}
async fn auth_callback(State(s):State<Arc<AppState>>,Query(q):Query<CallbackQuery>,headers:HeaderMap)->Response {
    if q.error.is_some() {return (StatusCode::UNAUTHORIZED,"Connexion refusée par Pocket ID").into_response();}
    let (Some(code),Some(state),Some(browser))=(q.code,q.state,cookie(&headers,"todo_oidc")) else {return (StatusCode::BAD_REQUEST,"Callback OIDC incomplet").into_response()};
    let result:Result<String>=async {
        let flow=sqlx::query_as::<_,AuthFlow>("DELETE FROM auth_flows WHERE state_hash=? AND expires_at>? RETURNING verifier,nonce,browser_hash")
            .bind(digest(&state)).bind(Utc::now().timestamp()).fetch_optional(&s.db).await?.context("State absent ou expiré")?;
        if digest(&browser)!=flow.browser_hash {bail!("Browser mismatch");}
        let response=s.http.post(&s.oidc.token_endpoint).basic_auth(&s.client_id,Some(&s.client_secret))
           .form(&[("grant_type","authorization_code"),("code",code.as_str()),("redirect_uri",&format!("{}/auth/callback",s.public_url)),("code_verifier",flow.verifier.as_str())])
           .send().await?.error_for_status()?;
        let tokens:TokenResponse=response.json().await?;
        let head=decode_header(&tokens.id_token)?;
        // Le fournisseur OIDC doit signer avec une clef asymétrique.
        use jsonwebtoken::Algorithm;
        if !matches!(head.alg,Algorithm::RS256|Algorithm::RS384|Algorithm::RS512|Algorithm::ES256|Algorithm::ES384) {bail!("Algorithme OIDC non supporté");}
        let jwks:JwkSet=s.http.get(&s.oidc.jwks_uri).send().await?.error_for_status()?.json().await?;
        let jwk=jwks.keys.iter().find(|k|k.common.key_id==head.kid).context("OIDC kid introuvable")?;
        let key=DecodingKey::from_jwk(jwk)?;
        let mut validation=Validation::new(head.alg);
        validation.set_audience(&[&s.client_id]);validation.set_issuer(&[&s.issuer]);
        let claims:Claims=decode::<Claims>(&tokens.id_token,&key,&validation)?.claims;
        if claims.nonce!=flow.nonce || claims.sub.is_empty() || claims.exp==0 {bail!("OIDC nonce/sub invalide");}
        let name=claims.name.or(claims.preferred_username).or(claims.email).unwrap_or_else(||"Utilisateur".into());
        sqlx::query("INSERT INTO users(sub,name) VALUES (?,?) ON CONFLICT(sub) DO UPDATE SET name=excluded.name")
           .bind(&claims.sub).bind(&name).execute(&s.db).await?;
        let token=random_url_token();
        sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,?,?)")
           .bind(digest(&token)).bind(&claims.sub).bind(Utc::now().timestamp()+7*86400).execute(&s.db).await?;
        Ok(token)
    }.await;
    match result {
        Ok(token)=> {let mut r=Redirect::to("/").into_response();r.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_session",&token,7*86400)).unwrap());r.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_oidc","",0)).unwrap());add_security_headers(r)}
        Err(err)=>{error!("OIDC callback: {err:#}");(StatusCode::UNAUTHORIZED,"Échec OIDC. Vérifier la configuration Pocket ID et les journaux du conteneur.").into_response()}
    }
}
async fn auth_logout(State(s):State<Arc<AppState>>,headers:HeaderMap)->Response {
    if let Some(token)=cookie(&headers,"todo_session") {let _=sqlx::query("DELETE FROM sessions WHERE token_hash=?").bind(digest(&token)).execute(&s.db).await;}
    let mut r=Redirect::to("/").into_response();r.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_session","",0)).unwrap());add_security_headers(r)
}
async fn list_tasks(State(s):State<Arc<AppState>>,h:HeaderMap)->ApiResult<Json<Vec<Task>>> {
    let user=require_user(&s,&h).await?;
    let tasks=sqlx::query_as::<_,Task>("SELECT id,parent_id,title,description,sort_order,completed_at,created_at FROM tasks WHERE owner_sub=? ORDER BY sort_order ASC,created_at ASC")
      .bind(&user.sub).fetch_all(&s.db).await.map_err(server_error)?;
    Ok(Json(tasks))
}
fn validate_task(t:&TaskInput)->ApiResult<()> {
    if t.title.trim().is_empty()||t.title.chars().count()>160 {return Err(api_error(StatusCode::BAD_REQUEST,"Le titre doit contenir entre 1 et 160 caractères"));}
    if t.description.chars().count()>10000 {return Err(api_error(StatusCode::BAD_REQUEST,"Description trop longue"));}
    Ok(())
}
async fn parent_valid(s:&AppState,sub:&str,id:&str)->ApiResult<bool> {
    sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks WHERE owner_sub=? AND id=?")
       .bind(sub).bind(id).fetch_one(&s.db).await.map(|n|n>0).map_err(server_error)
}
async fn create_task(State(s):State<Arc<AppState>>,h:HeaderMap,Json(t):Json<TaskInput>)->ApiResult<(StatusCode,Json<serde_json::Value>)> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;validate_task(&t)?;
    if let Some(ref parent)=t.parent_id {if !parent_valid(&s,&u.sub,parent).await? {return Err(api_error(StatusCode::BAD_REQUEST,"Tâche parent introuvable"));}}
    let order: i64=sqlx::query_scalar("SELECT COALESCE(MAX(sort_order),-1)+1 FROM tasks WHERE owner_sub=? AND parent_id IS ?")
       .bind(&u.sub).bind(&t.parent_id).fetch_one(&s.db).await.map_err(server_error)?;
    let id=uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,description,sort_order,created_at) VALUES (?,?,?,?,?,?,?)")
       .bind(&id).bind(&u.sub).bind(&t.parent_id).bind(t.title.trim()).bind(t.description.trim()).bind(order).bind(Utc::now().timestamp()).execute(&s.db).await.map_err(server_error)?;
    Ok((StatusCode::CREATED,Json(serde_json::json!({"id":id}))))
}
async fn edit_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(t):Json<TaskInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;validate_task(&t)?;
    // Le parent n'est pas modifiable à l'édition; pas de cycle possible.
    let res=sqlx::query("UPDATE tasks SET title=?,description=? WHERE id=? AND owner_sub=?")
      .bind(t.title.trim()).bind(t.description.trim()).bind(&id).bind(&u.sub).execute(&s.db).await.map_err(server_error)?;
    if res.rows_affected()==0 {return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn check_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(v):Json<CheckInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;
    let result=sqlx::query("UPDATE tasks SET completed_at=? WHERE id=? AND owner_sub=?")
      .bind(if v.done{Some(Utc::now().timestamp())}else{None}).bind(id).bind(u.sub).execute(&s.db).await.map_err(server_error)?;
    if result.rows_affected()==0{return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn delete_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap)->ApiResult<StatusCode> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;
    let r=sqlx::query("DELETE FROM tasks WHERE owner_sub=? AND id=?").bind(u.sub).bind(id).execute(&s.db).await.map_err(server_error)?;
    if r.rows_affected()==0 {return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    Ok(StatusCode::NO_CONTENT)
}
async fn move_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(v):Json<MoveInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;
    if v.direction!="up" && v.direction!="down" {return Err(api_error(StatusCode::BAD_REQUEST,"Direction invalide"));}
    let mut tx=s.db.begin().await.map_err(server_error)?;
    let current:Option<(Option<String>,i64)>=sqlx::query_as("SELECT parent_id,sort_order FROM tasks WHERE id=? AND owner_sub=?")
        .bind(&id).bind(&u.sub).fetch_optional(&mut *tx).await.map_err(server_error)?;
    let Some((parent,order))=current else{return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));};
    let other:Option<(String,i64)>=if v.direction=="up" {
        sqlx::query_as("SELECT id,sort_order FROM tasks WHERE owner_sub=? AND parent_id IS ? AND (sort_order<? OR (sort_order=? AND id<?)) ORDER BY sort_order DESC,id DESC LIMIT 1")
            .bind(&u.sub).bind(&parent).bind(order).bind(order).bind(&id).fetch_optional(&mut *tx).await.map_err(server_error)?
    } else {
        sqlx::query_as("SELECT id,sort_order FROM tasks WHERE owner_sub=? AND parent_id IS ? AND (sort_order>? OR (sort_order=? AND id>?)) ORDER BY sort_order ASC,id ASC LIMIT 1")
            .bind(&u.sub).bind(&parent).bind(order).bind(order).bind(&id).fetch_optional(&mut *tx).await.map_err(server_error)?
    };
    if let Some((other_id,other_order))=other {
        // Renumérote uniquement les frères pour éviter les collisions et les égalités de rang.
        let sibling_ids:Vec<String>=sqlx::query_scalar("SELECT id FROM tasks WHERE owner_sub=? AND parent_id IS ? ORDER BY sort_order,id")
            .bind(&u.sub).bind(&parent).fetch_all(&mut *tx).await.map_err(server_error)?;
        let mut ordered=sibling_ids;
        let a=ordered.iter().position(|x|x==&id).unwrap();let b=ordered.iter().position(|x|x==&other_id).unwrap();ordered.swap(a,b);
        for (pos,task_id) in ordered.iter().enumerate() {
            sqlx::query("UPDATE tasks SET sort_order=? WHERE owner_sub=? AND id=?").bind(pos as i64).bind(&u.sub).bind(task_id).execute(&mut *tx).await.map_err(server_error)?;
        }
        let _=other_order;
    }
    tx.commit().await.map_err(server_error)?;
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn init_db(db:&SqlitePool)->Result<()> {
    sqlx::query("PRAGMA foreign_keys=ON").execute(db).await?;
    for sql in [
      "CREATE TABLE IF NOT EXISTS users (sub TEXT PRIMARY KEY, name TEXT NOT NULL)",
      "CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY, user_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, expires_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS auth_flows (state_hash TEXT PRIMARY KEY, browser_hash TEXT NOT NULL, verifier TEXT NOT NULL, nonce TEXT NOT NULL, expires_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, owner_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, parent_id TEXT REFERENCES tasks(id) ON DELETE CASCADE, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', sort_order INTEGER NOT NULL DEFAULT 0, completed_at INTEGER, created_at INTEGER NOT NULL)",
      "CREATE INDEX IF NOT EXISTS idx_tasks_owner_parent_sort ON tasks(owner_sub,parent_id,sort_order)",
      "CREATE INDEX IF NOT EXISTS idx_tasks_completed ON tasks(completed_at)",
      "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at)",
    ] {sqlx::query(sql).execute(db).await?;}
    Ok(())
}
async fn cleanup(db:&SqlitePool)->Result<()> {
    let now=Utc::now().timestamp();
    // Les enfants d'une tâche supprimée sont supprimés en cascade, même non cochés.
    sqlx::query("DELETE FROM tasks WHERE completed_at IS NOT NULL AND completed_at<=?").bind(now-86400).execute(db).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at<=?").bind(now).execute(db).await?;
    sqlx::query("DELETE FROM auth_flows WHERE expires_at<=?").bind(now).execute(db).await?;
    Ok(())
}
#[tokio::main]
async fn main()->Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_|"info".into())).init();
    let public_url=env::var("PUBLIC_URL")?.trim_end_matches('/').to_owned();
    let issuer=env::var("OIDC_ISSUER_URL")?.trim_end_matches('/').to_owned();
    let client_id=env::var("OIDC_CLIENT_ID")?;let client_secret=env::var("OIDC_CLIENT_SECRET")?;
    let test_mode=env::var("APP_ENV").map(|v|v=="test").unwrap_or(false);
    let valid_url=|url:&str|url.starts_with("https://")||(test_mode&&url.starts_with("http://"));
    if !valid_url(&public_url)||!valid_url(&issuer) {bail!("PUBLIC_URL et OIDC_ISSUER_URL doivent être en HTTPS");}
    let port:u16=env::var("APP_PORT").unwrap_or_else(|_|"3080".into()).parse()?;
    let db_url=env::var("DATABASE_URL").unwrap_or_else(|_|"sqlite:///data/todo.db".into());
    let opt=SqliteConnectOptions::from_str(&db_url)?.create_if_missing(true).foreign_keys(true).journal_mode(sqlx::sqlite::SqliteJournalMode::Wal).busy_timeout(Duration::from_secs(5));
    let db=SqlitePoolOptions::new().max_connections(5).connect_with(opt).await?;init_db(&db).await?;cleanup(&db).await?;
    let http=reqwest::Client::builder().timeout(Duration::from_secs(12)).build()?;
    let url=format!("{issuer}/.well-known/openid-configuration");
    let oidc:Discovery=http.get(&url).send().await.context("Pocket ID inaccessible : découverte OIDC")?.error_for_status()?.json().await?;
    if oidc.issuer!=issuer {bail!("Issuer OIDC différent de OIDC_ISSUER_URL");}
    for endpoint in [&oidc.authorization_endpoint,&oidc.token_endpoint,&oidc.jwks_uri] {if !valid_url(endpoint) {bail!("Endpoint OIDC non HTTPS");}}
    let s=Arc::new(AppState{db:db.clone(),http,oidc,issuer,client_id,client_secret,public_url});
    tokio::spawn(async move {let mut interval=tokio::time::interval(Duration::from_secs(60));loop {interval.tick().await;if let Err(e)=cleanup(&db).await {error!("Nettoyage: {e}");}}});
    let app=Router::new()
      .route("/",get(index)).route("/health",get(health))
      .route("/static/style.css",get(style)).route("/static/app.js",get(script))
      .route("/auth/login",get(auth_login)).route("/auth/callback",get(auth_callback)).route("/auth/logout",get(auth_logout))
      .route("/api/tasks",get(list_tasks).post(create_task))
      .route("/api/tasks/{id}",axum::routing::patch(edit_task).delete(delete_task))
      .route("/api/tasks/{id}/check",post(check_task)).route("/api/tasks/{id}/move",post(move_task))
      .with_state(s);
    let listener=tokio::net::TcpListener::bind(("0.0.0.0",port)).await?;
    info!("Todo démarre sur 0.0.0.0:{port}");
    axum::serve(listener,app).await?;
    Ok(())
}
