use std::{env, str::FromStr, sync::Arc, time::Duration};
use std::collections::{HashMap, HashSet};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post}, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jiff::{civil::Date, tz::TimeZone, Span, Timestamp};
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
struct Task { id: String, parent_id: Option<String>, title: String, description: String, category: String, category_color: String, recurrence: String, sort_order: i64, completed_at: Option<i64>, next_reset_at: Option<i64>, created_at: i64 }
#[derive(Debug, Serialize, FromRow)]
struct Activity { id: i64, kind: String, title: String, occurred_at: i64 }
#[derive(Debug, Serialize)]
struct UpcomingActivity { task_id: String, kind: &'static str, title: String, scheduled_at: i64, affected_count: usize }
#[derive(Debug, Deserialize)]
struct TaskInput { title: String, #[serde(default)] description: String, #[serde(default)] parent_id: Option<String>, #[serde(default)] category: String, #[serde(default)] category_color: String, #[serde(default)] recurrence: String, #[serde(default)] position: Option<i64> }
#[derive(Debug, Deserialize)]
struct CheckInput { done: bool }
#[derive(Debug, Deserialize)]
struct CategoryInput { category: String }
#[derive(Debug, Default, Deserialize)]
struct ShareImportInput { #[serde(default)] parent_id:Option<String>, #[serde(default)] new_parent_title:Option<String> }
#[derive(Debug, Default, Deserialize)]
struct ShareOptions { #[serde(default)] duration_seconds:Option<i64>, #[serde(default)] mode:String }
#[derive(Debug, Serialize, Deserialize, FromRow)]
struct SharedTask { id:String, parent_id:Option<String>, title:String, description:String, category:String, recurrence:String, sort_order:i64 }
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
    let secure=if env::var("APP_ENV").as_deref()==Ok("test") && env::var("PUBLIC_URL").unwrap_or_default().starts_with("http://") { "" } else { " Secure;" };
    format!("{name}={value}; Path=/; HttpOnly;{secure} SameSite=Lax; Max-Age={age}")
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
fn origin_allowed(origin:&str, public_url:&str, test_mode:bool)->bool {
    if origin==public_url {return true;}
    if !test_mode {return false;}
    let Ok(public)=Url::parse(public_url) else {return false};
    let Ok(candidate)=Url::parse(origin) else {return false};
    candidate.scheme()=="http" && matches!(candidate.host_str(),Some("localhost"|"127.0.0.1"|"[::1]")) && candidate.port_or_known_default()==public.port_or_known_default() && candidate.path()=="/" && candidate.query().is_none() && candidate.fragment().is_none() && candidate.username().is_empty() && candidate.password().is_none()
}
fn guard_write(h: &HeaderMap, s: &AppState) -> ApiResult<()> {
    if h.get("x-todo-request").and_then(|v|v.to_str().ok())!=Some("1") { return Err(api_error(StatusCode::FORBIDDEN,"Requête non autorisée")); }
    if let Some(origin)=h.get(header::ORIGIN) { if !origin_allowed(origin.to_str().unwrap_or(""),&s.public_url,env::var("APP_ENV").as_deref()==Ok("test")) { return Err(api_error(StatusCode::FORBIDDEN,"Origine non autorisée")); } }
    Ok(())
}
async fn current_user(s: &AppState, h: &HeaderMap) -> Option<User> {
    let sid=cookie(h,"todo_session")?;
    if sid.len()!=43 { return None; }
    sqlx::query_as::<_,User>("SELECT u.sub,u.name FROM sessions x JOIN users u ON u.sub=x.user_sub WHERE x.token_hash=? AND x.expires_at>?")
        .bind(digest(&sid)).bind(Timestamp::now().as_second()).fetch_optional(&s.db).await.ok().flatten()
}
async fn require_user(s: &AppState, h: &HeaderMap) -> ApiResult<User> {
    current_user(s,h).await.ok_or_else(||api_error(StatusCode::UNAUTHORIZED,"Connexion requise"))
}
async fn index(State(s):State<Arc<AppState>>,headers:HeaderMap)->Response {
    let Some(user)=current_user(&s,&headers).await else {
        let label=if env::var("APP_ENV").as_deref()==Ok("test") && env::var("TEST_LOGIN").as_deref()==Ok("1") {"Accéder au compte de test"} else {"Se connecter avec Pocket ID"};
        let content=format!("<main class=\"wrap\"><section class=\"panel login\"><div class=\"logo\">✓</div><h1>Mes tâches</h1><p>Une liste simple, privée, et toujours à portée de main.</p><a class=\"primary\" href=\"/auth/login\">{label}</a></section></main>");
        return add_security_headers(shell(&content,"Todo · Connexion").into_response());
    };
    let html=format!(r#"<main class="wrap"><header class="topbar"><div class="brand"><div class="logo">✓</div><div><div class="title">Todo</div><div class="sub">L'essentiel, simplement.</div></div></div><div class="person"><span>{}</span><a href="/auth/logout" class="link">Déconnexion</a></div></header><section class="panel"><div class="heading"><div><h1>Mes tâches</h1><div class="count" id="count">Chargement…</div></div><button class="new" id="new-task">+ Nouvelle tâche</button></div><div id="tree" class="tree"><div class="empty">Chargement…</div></div></section><p class="footer">Les tâches cochées non récurrentes sont supprimées après 24 heures, sauf dans une branche récurrente · Espace personnel</p></main><div class="overlay hidden" id="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><form class="modal" id="task-form"><h2 id="modal-title">Nouvelle tâche</h2><input type="hidden" name="id"><input type="hidden" name="parent_id"><label class="field" for="task-title">Titre *</label><input class="fieldinput" id="task-title" name="title" maxlength="160" required autofocus placeholder="Que faut-il faire ?"><label class="field" for="task-description">Description (facultative)</label><textarea class="fieldinput fieldtextarea" id="task-description" name="description" maxlength="10000" placeholder="Quelques détails utiles…"></textarea><div class="modal-actions"><button class="muted" id="cancel" type="button">Annuler</button><button class="primary" type="submit">Enregistrer</button></div></form></div>"#,html_escape(&user.name));
    add_security_headers(shell(&html,"Todo · Mes tâches").into_response())
}
async fn style()->Response { let mut r=( [(header::CONTENT_TYPE,"text/css; charset=utf-8")],include_str!("../static/style.css") ).into_response();r.headers_mut().insert("x-content-type-options",HeaderValue::from_static("nosniff"));r }
async fn script()->Response { let mut r=( [(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../static/app.js") ).into_response();r.headers_mut().insert("x-content-type-options",HeaderValue::from_static("nosniff"));r }
async fn health()->impl IntoResponse {(StatusCode::OK,"ok")}
async fn auth_login(State(s):State<Arc<AppState>>)->Response {
    if env::var("APP_ENV").as_deref()==Ok("test") && env::var("TEST_LOGIN").as_deref()==Ok("1") {
        let result:Result<String>=async {
            sqlx::query("INSERT OR IGNORE INTO users(sub,name) VALUES ('local-test','Utilisateur test')").execute(&s.db).await?;
            seed_test_data(&s.db).await?;
            let token=random_url_token();
            sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,'local-test',?)").bind(digest(&token)).bind(Timestamp::now().as_second()+7*86400).execute(&s.db).await?;
            Ok(token)
        }.await;
        return match result {Ok(token)=>{let mut r=Redirect::to("/").into_response();r.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_session",&token,7*86400)).unwrap());add_security_headers(r)},Err(e)=>{error!("Test login: {e}");StatusCode::INTERNAL_SERVER_ERROR.into_response()}};
    }
    let state=random_url_token();let nonce=random_url_token();let verifier=random_url_token();let browser=random_url_token();
    let challenge=URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let saved=sqlx::query("INSERT INTO auth_flows (state_hash,browser_hash,verifier,nonce,expires_at) VALUES (?,?,?,?,?)")
       .bind(digest(&state)).bind(digest(&browser)).bind(&verifier).bind(&nonce).bind(Timestamp::now().as_second()+600).execute(&s.db).await;
    if let Err(e)=saved {error!("OIDC flow save: {e}");return StatusCode::INTERNAL_SERVER_ERROR.into_response();}
    let mut u=match Url::parse(&s.oidc.authorization_endpoint) {Ok(u)=>u,Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response()};
    u.query_pairs_mut().append_pair("response_type","code").append_pair("client_id",&s.client_id)
        .append_pair("redirect_uri",&format!("{}/auth/callback",s.public_url)).append_pair("scope","openid profile email")
        .append_pair("state",&state).append_pair("nonce",&nonce).append_pair("code_challenge",&challenge).append_pair("code_challenge_method","S256");
    let mut resp=Redirect::to(u.as_str()).into_response();
    resp.headers_mut().append(header::SET_COOKIE,HeaderValue::from_str(&set_cookie("todo_oidc",&browser,600)).unwrap());
    add_security_headers(resp)
}
async fn seed_test_data(db:&SqlitePool)->Result<()> {
    let already_seeded:i64=sqlx::query_scalar("SELECT COUNT(*) FROM app_meta WHERE key='test_data_v1'").fetch_one(db).await?;
    if already_seeded>0{return Ok(());}
    let now=Timestamp::now().as_second();
    let mut tx=db.begin().await?;
    let rows=[
        ("test-home",None,"Maison","Pièces et entretien de la maison.","maison","none",0,false),
        ("test-home-clean",Some("test-home"),"Nettoyer la cuisine","", "maison","none",0,false),
        ("test-home-shop",Some("test-home"),"Acheter des sacs poubelle","", "courses","none",1,true),
        ("test-gifts",None,"Liste de cadeaux","Exemple de liste partageable avec des proches.","cadeaux","none",1,false),
        ("test-gift-book",Some("test-gifts"),"Un livre","Choisir un roman ou un essai.","cadeaux","none",0,false),
        ("test-gift-headphones",Some("test-gifts"),"Casque audio","", "cadeaux","none",1,false),
        ("test-routine",None,"Routine quotidienne","Cette tâche se réinitialise automatiquement.","routine","daily",2,false),
        ("test-routine-water",Some("test-routine"),"Boire suffisamment d'eau","", "santé","none",0,false),
        ("test-done",None,"Tâche terminée de démonstration","Elle permet de voir l'état coché.","demo","none",3,true),
    ];
    for (id,parent,title,description,category,recurrence,sort_order,done) in rows {
        sqlx::query("INSERT OR IGNORE INTO tasks(id,owner_sub,parent_id,title,description,category,category_color,recurrence,sort_order,completed_at,next_reset_at,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(id).bind("local-test").bind(parent).bind(title).bind(description).bind(category).bind("").bind(recurrence).bind(sort_order)
            .bind(if done {Some(now)} else {None}).bind(recurrence_after(now,recurrence)).bind(now).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO app_meta(key) VALUES ('test_data_v1')").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
async fn auth_callback(State(s):State<Arc<AppState>>,Query(q):Query<CallbackQuery>,headers:HeaderMap)->Response {
    if q.error.is_some() {return (StatusCode::UNAUTHORIZED,"Connexion refusée par Pocket ID").into_response();}
    let (Some(code),Some(state),Some(browser))=(q.code,q.state,cookie(&headers,"todo_oidc")) else {return (StatusCode::BAD_REQUEST,"Callback OIDC incomplet").into_response()};
    let result:Result<String>=async {
        let flow=sqlx::query_as::<_,AuthFlow>("DELETE FROM auth_flows WHERE state_hash=? AND expires_at>? RETURNING verifier,nonce,browser_hash")
            .bind(digest(&state)).bind(Timestamp::now().as_second()).fetch_optional(&s.db).await?.context("State absent ou expiré")?;
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
           .bind(digest(&token)).bind(&claims.sub).bind(Timestamp::now().as_second()+7*86400).execute(&s.db).await?;
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
    let mut tasks=sqlx::query_as::<_,Task>("WITH RECURSIVE visible(id) AS (SELECT id FROM tasks WHERE owner_sub=? UNION SELECT st.root_task_id FROM shared_tasks st JOIN share_access sa ON sa.token_hash=st.token_hash WHERE sa.user_sub=? AND st.root_task_id IS NOT NULL AND (st.expires_at IS NULL OR st.expires_at>?) UNION SELECT t.id FROM tasks t JOIN visible v ON t.parent_id=v.id) SELECT id,parent_id,title,description,category,category_color,recurrence,sort_order,completed_at,next_reset_at,created_at FROM tasks WHERE id IN (SELECT id FROM visible) ORDER BY sort_order ASC,created_at ASC")
      .bind(&user.sub).bind(&user.sub).bind(Timestamp::now().as_second()).fetch_all(&s.db).await.map_err(server_error)?;
    let visible_ids:HashSet<String>=tasks.iter().map(|task|task.id.clone()).collect();
    for task in &mut tasks {
        task.category_color=category_color(&task.category).to_string();
        if task.parent_id.as_ref().is_some_and(|parent|!visible_ids.contains(parent)) {task.parent_id=None;}
    }
    Ok(Json(tasks))
}
fn category_color(category:&str)->u32 {
    if category.is_empty(){return 0;}
    let hash=category.as_bytes().iter().fold(2166136261u32,|hash,byte|(hash^u32::from(*byte)).wrapping_mul(16777619));
    hash%10+1
}
async fn list_activity(State(s):State<Arc<AppState>>,h:HeaderMap)->ApiResult<Json<Vec<Activity>>> {
    let user=require_user(&s,&h).await?;
    let events=sqlx::query_as::<_,Activity>("SELECT id,kind,title,occurred_at FROM activity WHERE owner_sub=? ORDER BY occurred_at DESC,id DESC LIMIT 200").bind(user.sub).fetch_all(&s.db).await.map_err(server_error)?;
    Ok(Json(events))
}
fn upcoming_activity(tasks:&[Task])->Vec<UpcomingActivity> {
    let by_id:HashMap<&str,&Task>=tasks.iter().map(|task|(task.id.as_str(),task)).collect();
    let mut children:HashMap<&str,Vec<&Task>>=HashMap::new();
    for task in tasks {if let Some(parent)=&task.parent_id {children.entry(parent).or_default().push(task);}}
    let mut protected=HashSet::new();
    // Une récurrence protège à la fois sa descendance et ses ancêtres des suppressions.
    for task in tasks.iter().filter(|task|task.recurrence!="none") {
        let mut ancestor=Some(task);
        while let Some(current)=ancestor {if !protected.insert(current.id.as_str()){break;}ancestor=current.parent_id.as_deref().and_then(|id|by_id.get(id).copied());}
    }
    let mut visited=HashSet::new();
    let mut stack:Vec<&Task>=tasks.iter().filter(|task|task.recurrence!="none").collect();
    while let Some(task)=stack.pop() {if !visited.insert(task.id.as_str()){continue;}protected.insert(task.id.as_str());if let Some(branch)=children.get(task.id.as_str()){stack.extend(branch);}}
    let mut events=Vec::new();
    let mut resets:Vec<&Task>=tasks.iter().filter(|task|task.recurrence!="none"&&task.next_reset_at.is_some()).collect();
    resets.sort_by_key(|task|(task.next_reset_at.unwrap(),task.id.as_str()));
    let mut checked:HashSet<&str>=tasks.iter().filter(|task|task.completed_at.is_some()).map(|task|task.id.as_str()).collect();
    for task in resets {
        let Some(scheduled_at)=task.next_reset_at else {continue};
        let mut affected_count=0;let mut branch=vec![task];
        while let Some(item)=branch.pop() {affected_count+=usize::from(checked.remove(item.id.as_str()));if let Some(descendants)=children.get(item.id.as_str()){branch.extend(descendants);}}
        events.push(UpcomingActivity{task_id:task.id.clone(),kind:"reset",title:task.title.clone(),scheduled_at,affected_count});
    }
    let mut deletions:Vec<&Task>=tasks.iter().filter(|task|task.completed_at.is_some()&&!protected.contains(task.id.as_str())).collect();
    // À date égale, traiter le parent avant son enfant pour regrouper la cascade.
    let depth=|task:&Task|{let mut count=0;let mut parent=task.parent_id.as_deref();while let Some(id)=parent {count+=1;parent=by_id.get(id).and_then(|task|task.parent_id.as_deref());}count};
    deletions.sort_by_key(|task|(task.completed_at.unwrap(),depth(task),task.id.as_str()));
    let mut removed=HashSet::new();
    for task in deletions {
        if removed.contains(task.id.as_str()){continue;}
        let mut affected_count=0;let mut branch=vec![task];
        while let Some(item)=branch.pop() {if !removed.insert(item.id.as_str()){continue;}affected_count+=1;if let Some(descendants)=children.get(item.id.as_str()){branch.extend(descendants);}}
        events.push(UpcomingActivity{task_id:task.id.clone(),kind:"deleted",title:task.title.clone(),scheduled_at:task.completed_at.unwrap().saturating_add(86400),affected_count});
    }
    events.sort_by(|a,b|a.scheduled_at.cmp(&b.scheduled_at).then_with(||a.task_id.cmp(&b.task_id)));
    events
}
async fn list_upcoming_activity(State(s):State<Arc<AppState>>,h:HeaderMap)->ApiResult<Json<serde_json::Value>> {
    let user=require_user(&s,&h).await?;
    let tasks=sqlx::query_as::<_,Task>("SELECT id,parent_id,title,description,category,category_color,recurrence,sort_order,completed_at,next_reset_at,created_at FROM tasks WHERE owner_sub=?")
        .bind(user.sub).fetch_all(&s.db).await.map_err(server_error)?;
    Ok(Json(serde_json::json!({"events":upcoming_activity(&tasks),"timezone":env::var("TODO_TIMEZONE").unwrap_or_else(|_|"Europe/Paris".into())})))
}
fn validate_task(t:&TaskInput)->ApiResult<()> {
    if t.title.trim().is_empty()||t.title.chars().count()>160 {return Err(api_error(StatusCode::BAD_REQUEST,"Le titre doit contenir entre 1 et 160 caractères"));}
    if t.description.chars().count()>10000 {return Err(api_error(StatusCode::BAD_REQUEST,"Description trop longue"));}
    if t.category.chars().count()>40 {return Err(api_error(StatusCode::BAD_REQUEST,"Catégorie trop longue"));}
    if !["none","daily","weekly","monthly","quarterly","yearly"].contains(&t.recurrence.as_str()) {return Err(api_error(StatusCode::BAD_REQUEST,"Récurrence invalide"));}
    Ok(())
}
async fn create_task(State(s):State<Arc<AppState>>,h:HeaderMap,Json(t):Json<TaskInput>)->ApiResult<(StatusCode,Json<serde_json::Value>)> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;validate_task(&t)?;
    let mut tx=s.db.begin_with("BEGIN IMMEDIATE").await.map_err(server_error)?;
    if let Some(ref parent)=t.parent_id {let valid:i64=sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE owner_sub=? AND id=?").bind(&u.sub).bind(parent).fetch_one(&mut *tx).await.map_err(server_error)?;if valid==0 {return Err(api_error(StatusCode::BAD_REQUEST,"Tâche parent introuvable"));}}
    let max_order:i64=sqlx::query_scalar("SELECT COALESCE(MAX(sort_order),-1) FROM tasks WHERE owner_sub=? AND parent_id IS ?")
       .bind(&u.sub).bind(&t.parent_id).fetch_one(&mut *tx).await.map_err(server_error)?;
    let order=t.position.unwrap_or(max_order+1).max(0).min(max_order+1);
    let color=if t.category_color.trim().is_empty() { "".to_string() } else { t.category_color.trim().to_string() };
    let id=uuid::Uuid::new_v4().to_string();
    let now=Timestamp::now().as_second();
    sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,description,category,category_color,recurrence,sort_order,created_at,next_reset_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)")
       .bind(&id).bind(&u.sub).bind(&t.parent_id).bind(t.title.trim()).bind(t.description.trim()).bind(t.category.trim().to_lowercase()).bind(color).bind(&t.recurrence).bind(order).bind(now).bind(recurrence_after(now,&t.recurrence)).execute(&mut *tx).await.map_err(server_error)?;
    if t.position.is_some() {set_position_in(&mut tx,&u.sub,&id,order).await?;}
    tx.commit().await.map_err(server_error)?;
    Ok((StatusCode::CREATED,Json(serde_json::json!({"id":id}))))
}
async fn edit_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(t):Json<TaskInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;validate_task(&t)?;
    let mut tx=s.db.begin_with("BEGIN IMMEDIATE").await.map_err(server_error)?;
    // Le parent n'est pas modifiable à l'édition; pas de cycle possible.
    let previous:Option<(Option<i64>,String,Option<i64>)>=sqlx::query_as("SELECT completed_at,recurrence,next_reset_at FROM tasks WHERE id=? AND owner_sub=?").bind(&id).bind(&u.sub).fetch_optional(&mut *tx).await.map_err(server_error)?;
    let Some((completed,old_recurrence,old_reset))=previous else {return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));};
    let next=if old_recurrence==t.recurrence {old_reset} else {recurrence_after(Timestamp::now().as_second(),&t.recurrence)};
    let _=completed;
    let res=sqlx::query("UPDATE tasks SET title=?,description=?,category=?,category_color=?,recurrence=?,next_reset_at=? WHERE id=? AND owner_sub=?")
      .bind(t.title.trim()).bind(t.description.trim()).bind(t.category.trim().to_lowercase()).bind(if t.category_color.trim().is_empty(){""}else{t.category_color.trim()}).bind(&t.recurrence).bind(next).bind(&id).bind(&u.sub).execute(&mut *tx).await.map_err(server_error)?;
    if let Some(position)=t.position { set_position_in(&mut tx,&u.sub,&id,position).await?; }
    if res.rows_affected()==0 {return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    tx.commit().await.map_err(server_error)?;
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn set_category(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(input):Json<CategoryInput>)->ApiResult<StatusCode> {
    guard_write(&h,&s)?;let user=require_user(&s,&h).await?;
    let category=input.category.trim().to_lowercase();
    if category.chars().count()>40 {return Err(api_error(StatusCode::BAD_REQUEST,"Catégorie trop longue"));}
    let result=sqlx::query("UPDATE tasks SET category=? WHERE id=? AND owner_sub=?").bind(category).bind(id).bind(user.sub).execute(&s.db).await.map_err(server_error)?;
    if result.rows_affected()==0 {return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    Ok(StatusCode::NO_CONTENT)
}
async fn create_share(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(options):Json<ShareOptions>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let user=require_user(&s,&h).await?;
    if !matches!(options.duration_seconds,None|Some(86400)|Some(604800)){return Err(api_error(StatusCode::BAD_REQUEST,"Durée invalide"));}
    if !options.mode.is_empty() && options.mode!="copy" && options.mode!="live" {return Err(api_error(StatusCode::BAD_REQUEST,"Mode de partage invalide"));}
    let tasks=sqlx::query_as::<_,SharedTask>("WITH RECURSIVE branch(id,depth) AS (SELECT id,0 FROM tasks WHERE id=? AND owner_sub=? UNION ALL SELECT t.id,b.depth+1 FROM tasks t JOIN branch b ON t.parent_id=b.id WHERE t.owner_sub=?) SELECT t.id,CASE WHEN b.depth=0 THEN NULL ELSE t.parent_id END AS parent_id,t.title,t.description,t.category,t.recurrence,t.sort_order FROM branch b JOIN tasks t ON t.id=b.id ORDER BY b.depth,t.sort_order,t.created_at LIMIT 1001")
        .bind(&id).bind(&user.sub).bind(&user.sub).fetch_all(&s.db).await.map_err(server_error)?;
    if tasks.is_empty(){return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    if tasks.len()>1000{return Err(api_error(StatusCode::BAD_REQUEST,"Ce partage contient trop de tâches"));}
    let token=random_url_token();let snapshot=serde_json::to_string(&tasks).map_err(server_error)?;
    let now=Timestamp::now().as_second();let expires=options.duration_seconds.map(|duration|now+duration);
    let root_task_id=if options.mode=="live" {Some(id.as_str())} else {None};
    sqlx::query("INSERT INTO shared_tasks(token_hash,owner_sub,root_task_id,snapshot,created_at,expires_at) VALUES (?,?,?,?,?,?)").bind(digest(&token)).bind(user.sub).bind(root_task_id).bind(snapshot).bind(now).bind(expires).execute(&s.db).await.map_err(server_error)?;
    Ok(Json(serde_json::json!({"token":token,"expires_at":expires,"live":options.mode=="live"})))
}
async fn share_snapshot(s:&AppState,token:&str)->ApiResult<Vec<SharedTask>> {
    if token.len()!=43{return Err(api_error(StatusCode::NOT_FOUND,"Lien de partage introuvable"));}
    let share:Option<(String,Option<String>)>=sqlx::query_as("SELECT snapshot,root_task_id FROM shared_tasks WHERE token_hash=? AND (expires_at IS NULL OR expires_at>?)").bind(digest(token)).bind(Timestamp::now().as_second()).fetch_optional(&s.db).await.map_err(server_error)?;
    let (snapshot,root)=share.ok_or_else(||api_error(StatusCode::NOT_FOUND,"Lien de partage expiré ou désactivé"))?;
    if let Some(root)=root {
        return sqlx::query_as::<_,SharedTask>("WITH RECURSIVE branch(id,depth) AS (SELECT id,0 FROM tasks WHERE id=? UNION ALL SELECT t.id,b.depth+1 FROM tasks t JOIN branch b ON t.parent_id=b.id) SELECT t.id,CASE WHEN b.depth=0 THEN NULL ELSE t.parent_id END AS parent_id,t.title,t.description,t.category,t.recurrence,t.sort_order FROM branch b JOIN tasks t ON t.id=b.id ORDER BY b.depth,t.sort_order,t.created_at")
            .bind(root).fetch_all(&s.db).await.map_err(server_error);
    }
    serde_json::from_str(&snapshot).map_err(server_error)
}
async fn preview_share(State(s):State<Arc<AppState>>,Path(token):Path<String>,h:HeaderMap)->ApiResult<Json<serde_json::Value>> {
    let user=require_user(&s,&h).await?;let tasks=share_snapshot(&s,&token).await?;
    let now=Timestamp::now().as_second();
    sqlx::query("INSERT OR IGNORE INTO share_access(token_hash,user_sub,opened_at) SELECT token_hash,?,? FROM shared_tasks WHERE token_hash=? AND root_task_id IS NOT NULL AND (expires_at IS NULL OR expires_at>?)")
        .bind(&user.sub).bind(now).bind(digest(&token)).bind(now).execute(&s.db).await.map_err(server_error)?;
    let imported:i64=sqlx::query_scalar("SELECT COUNT(*) FROM share_imports WHERE token_hash=? AND user_sub=?").bind(digest(&token)).bind(user.sub).fetch_one(&s.db).await.map_err(server_error)?;
    let live:i64=sqlx::query_scalar("SELECT COUNT(*) FROM shared_tasks WHERE token_hash=? AND root_task_id IS NOT NULL").bind(digest(&token)).fetch_one(&s.db).await.map_err(server_error)?;
    Ok(Json(serde_json::json!({"tasks":tasks,"already_imported":imported>0,"live":live>0})))
}
async fn list_shares(State(s):State<Arc<AppState>>,h:HeaderMap)->ApiResult<Json<Vec<serde_json::Value>>> {
    let user=require_user(&s,&h).await?;
    // Ne pas transférer et désérialiser chaque copie complète pour afficher sa liste.
    let rows:Vec<(String,String,i64,Option<i64>,i64,bool)>=sqlx::query_as("SELECT token_hash,COALESCE(json_extract(snapshot,'$[0].title'),''),created_at,expires_at,json_array_length(snapshot),root_task_id IS NOT NULL FROM shared_tasks WHERE owner_sub=? ORDER BY created_at DESC,token_hash")
        .bind(user.sub).fetch_all(&s.db).await.map_err(server_error)?;
    Ok(Json(rows.into_iter().map(|(id,title,created_at,expires_at,task_count,live)|serde_json::json!({"id":id,"title":title,"created_at":created_at,"expires_at":expires_at,"task_count":task_count,"live":live})).collect()))
}
async fn revoke_share(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap)->ApiResult<StatusCode> {
    guard_write(&h,&s)?;let user=require_user(&s,&h).await?;let result=sqlx::query("DELETE FROM shared_tasks WHERE token_hash=? AND owner_sub=?").bind(id).bind(user.sub).execute(&s.db).await.map_err(server_error)?;if result.rows_affected()==0{return Err(api_error(StatusCode::NOT_FOUND,"Partage introuvable"));}Ok(StatusCode::NO_CONTENT)
}
async fn import_share(State(s):State<Arc<AppState>>,Path(token):Path<String>,h:HeaderMap,Json(input):Json<ShareImportInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let user=require_user(&s,&h).await?;let tasks=share_snapshot(&s,&token).await?;
    if input.parent_id.is_some()&&input.new_parent_title.is_some(){return Err(api_error(StatusCode::BAD_REQUEST,"Choisis une seule destination"));}
    if let Some(title)=&input.new_parent_title {if title.trim().is_empty()||title.trim().chars().count()>160{return Err(api_error(StatusCode::BAD_REQUEST,"Le titre de la tâche parente doit contenir entre 1 et 160 caractères"));}}
    let mut tx=s.db.begin_with("BEGIN IMMEDIATE").await.map_err(server_error)?;
    let valid:i64=sqlx::query_scalar("SELECT COUNT(*) FROM shared_tasks WHERE token_hash=? AND (expires_at IS NULL OR expires_at>?)").bind(digest(&token)).bind(Timestamp::now().as_second()).fetch_one(&mut *tx).await.map_err(server_error)?;if valid==0{return Err(api_error(StatusCode::NOT_FOUND,"Lien de partage expiré ou désactivé"));}
    if let Some(parent)=&input.parent_id {let exists:i64=sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE id=? AND owner_sub=?").bind(parent).bind(&user.sub).fetch_one(&mut *tx).await.map_err(server_error)?;if exists==0{return Err(api_error(StatusCode::BAD_REQUEST,"Tâche parente introuvable"));}}
    let now=Timestamp::now().as_second();let mut destination=input.parent_id;
    if let Some(title)=input.new_parent_title {let id=uuid::Uuid::new_v4().to_string();let order:i64=sqlx::query_scalar("SELECT COALESCE(MAX(sort_order),-1)+1 FROM tasks WHERE owner_sub=? AND parent_id IS NULL").bind(&user.sub).fetch_one(&mut *tx).await.map_err(server_error)?;sqlx::query("INSERT INTO tasks(id,owner_sub,title,sort_order,created_at) VALUES (?,?,?,?,?)").bind(&id).bind(&user.sub).bind(title.trim()).bind(order).bind(now).execute(&mut *tx).await.map_err(server_error)?;destination=Some(id);}
    let root_order:i64=sqlx::query_scalar("SELECT COALESCE(MAX(sort_order),-1)+1 FROM tasks WHERE owner_sub=? AND parent_id IS ?").bind(&user.sub).bind(&destination).fetch_one(&mut *tx).await.map_err(server_error)?;
    let mut ids=HashMap::new();let mut root_id=String::new();
    for task in tasks {let id=uuid::Uuid::new_v4().to_string();let is_root=task.parent_id.is_none();let parent=match &task.parent_id {Some(old)=>Some(ids.get(old).cloned().ok_or_else(||api_error(StatusCode::INTERNAL_SERVER_ERROR,"Partage invalide"))?),None=>{root_id=id.clone();destination.clone()}};
        sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,description,category,recurrence,sort_order,created_at,next_reset_at) VALUES (?,?,?,?,?,?,?,?,?,?)")
          .bind(&id).bind(&user.sub).bind(&parent).bind(&task.title).bind(&task.description).bind(&task.category).bind(&task.recurrence).bind(if is_root{root_order}else{task.sort_order}).bind(now).bind(recurrence_after(now,&task.recurrence)).execute(&mut *tx).await.map_err(server_error)?;
        ids.insert(task.id,id);
    }
    sqlx::query("INSERT OR IGNORE INTO share_imports(token_hash,user_sub) VALUES (?,?)").bind(digest(&token)).bind(&user.sub).execute(&mut *tx).await.map_err(server_error)?;
    tx.commit().await.map_err(server_error)?;Ok(Json(serde_json::json!({"id":root_id})))
}
fn recurrence_after(now:i64, recurrence:&str)->Option<i64> {
    let tz=TimeZone::get(&env::var("TODO_TIMEZONE").unwrap_or_else(|_|"Europe/Paris".into())).ok()?;
    recurrence_after_in(now,recurrence,&tz)
}
fn recurrence_after_in(now:i64, recurrence:&str, tz:&TimeZone)->Option<i64> {
    let local=Timestamp::from_second(now).ok()?.to_zoned_datetime(tz.clone());
    let date=local.date();
    let next=match recurrence {
        "daily"=>date.checked_add(Span::new().days(1)).ok()?,
        "weekly"=>date.checked_add(Span::new().days(7-date.weekday().to_monday_zero_offset())).ok()?,
        "monthly"=>if date.month()==12 {Date::new(date.year()+1,1,1).ok()?} else {Date::new(date.year(),date.month()+1,1).ok()?},
        "quarterly"=>{let month=((date.month()-1)/3+1)*3+1;if month>12 {Date::new(date.year()+1,1,1).ok()?} else {Date::new(date.year(),month,1).ok()?}},
        "yearly"=>Date::new(date.year()+1,1,1).ok()?,
        _=>return None,
    };
    Some(next.to_zoned_datetime(tz.clone()).ok()?.timestamp().as_second())
}
async fn set_position(s:&AppState,sub:&str,id:&str,position:i64)->ApiResult<()> {
    let mut tx=s.db.begin_with("BEGIN IMMEDIATE").await.map_err(server_error)?;
    set_position_in(&mut tx,sub,id,position).await?;
    tx.commit().await.map_err(server_error)?;
    Ok(())
}
async fn set_position_in(tx:&mut sqlx::Transaction<'_,sqlx::Sqlite>,sub:&str,id:&str,position:i64)->ApiResult<()> {
    let Some((parent,)):Option<(Option<String>,)>=sqlx::query_as("SELECT parent_id FROM tasks WHERE id=? AND owner_sub=?").bind(id).bind(sub).fetch_optional(&mut **tx).await.map_err(server_error)? else { return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable")); };
    let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM tasks WHERE owner_sub=? AND parent_id IS ? ORDER BY sort_order,id").bind(sub).bind(&parent).fetch_all(&mut **tx).await.map_err(server_error)?;
    let mut ordered=ids.into_iter().filter(|x|x!=id).collect::<Vec<_>>();
    let pos=position.max(0).min(ordered.len() as i64) as usize; ordered.insert(pos,id.to_string());
    for (index,task_id) in ordered.iter().enumerate() { sqlx::query("UPDATE tasks SET sort_order=? WHERE owner_sub=? AND id=?").bind(index as i64).bind(sub).bind(task_id).execute(&mut **tx).await.map_err(server_error)?; }
    Ok(())
}
async fn check_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(v):Json<CheckInput>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?;let u=require_user(&s,&h).await?;
    // Traiter une période échue avant une nouvelle validation juste après minuit.
    cleanup(&s.db).await.map_err(server_error)?;
    let now=Timestamp::now().as_second();
    let recurrence:Option<String>=sqlx::query_scalar("WITH RECURSIVE visible(id) AS (SELECT id FROM tasks WHERE owner_sub=? UNION SELECT st.root_task_id FROM shared_tasks st JOIN share_access sa ON sa.token_hash=st.token_hash WHERE sa.user_sub=? AND st.root_task_id IS NOT NULL AND (st.expires_at IS NULL OR st.expires_at>?) UNION SELECT t.id FROM tasks t JOIN visible v ON t.parent_id=v.id) SELECT recurrence FROM tasks WHERE id=? AND id IN (SELECT id FROM visible)").bind(&u.sub).bind(&u.sub).bind(now).bind(&id).fetch_optional(&s.db).await.map_err(server_error)?;
    if recurrence.is_none(){return Err(api_error(StatusCode::NOT_FOUND,"Tâche introuvable"));}
    let result=sqlx::query("WITH RECURSIVE visible(id) AS (SELECT id FROM tasks WHERE owner_sub=? UNION SELECT st.root_task_id FROM shared_tasks st JOIN share_access sa ON sa.token_hash=st.token_hash WHERE sa.user_sub=? AND st.root_task_id IS NOT NULL AND (st.expires_at IS NULL OR st.expires_at>?) UNION SELECT t.id FROM tasks t JOIN visible v ON t.parent_id=v.id) UPDATE tasks SET completed_at=? WHERE id=? AND id IN (SELECT id FROM visible)")
      .bind(&u.sub).bind(&u.sub).bind(now).bind(if v.done{Some(now)}else{None}).bind(id).execute(&s.db).await.map_err(server_error)?;
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
    let mut tx=s.db.begin_with("BEGIN IMMEDIATE").await.map_err(server_error)?;
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
async fn position_task(State(s):State<Arc<AppState>>,Path(id):Path<String>,h:HeaderMap,Json(v):Json<serde_json::Value>)->ApiResult<Json<serde_json::Value>> {
    guard_write(&h,&s)?; let u=require_user(&s,&h).await?;
    let position=v.get("position").and_then(|x|x.as_i64()).unwrap_or(0);
    set_position(&s,&u.sub,&id,position).await?;
    Ok(Json(serde_json::json!({"ok":true})))
}
async fn init_db(db:&SqlitePool)->Result<()> {
    sqlx::query("PRAGMA foreign_keys=ON").execute(db).await?;
    for sql in [
      "CREATE TABLE IF NOT EXISTS users (sub TEXT PRIMARY KEY, name TEXT NOT NULL)",
      "CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY, user_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, expires_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS auth_flows (state_hash TEXT PRIMARY KEY, browser_hash TEXT NOT NULL, verifier TEXT NOT NULL, nonce TEXT NOT NULL, expires_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, owner_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, parent_id TEXT REFERENCES tasks(id) ON DELETE CASCADE, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT '', category_color TEXT NOT NULL DEFAULT '#7c8cff', recurrence TEXT NOT NULL DEFAULT 'none', sort_order INTEGER NOT NULL DEFAULT 0, completed_at INTEGER, next_reset_at INTEGER, created_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS activity (id INTEGER PRIMARY KEY AUTOINCREMENT, owner_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, kind TEXT NOT NULL, title TEXT NOT NULL, occurred_at INTEGER NOT NULL)",
      "CREATE TABLE IF NOT EXISTS shared_tasks (token_hash TEXT PRIMARY KEY, owner_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, root_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE, snapshot TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT 0, expires_at INTEGER)",
      "CREATE TABLE IF NOT EXISTS share_imports (token_hash TEXT NOT NULL REFERENCES shared_tasks(token_hash) ON DELETE CASCADE, user_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, PRIMARY KEY(token_hash,user_sub))",
      "CREATE TABLE IF NOT EXISTS share_access (token_hash TEXT NOT NULL REFERENCES shared_tasks(token_hash) ON DELETE CASCADE, user_sub TEXT NOT NULL REFERENCES users(sub) ON DELETE CASCADE, opened_at INTEGER NOT NULL, PRIMARY KEY(token_hash,user_sub))",
      "CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY)",
      "CREATE INDEX IF NOT EXISTS idx_activity_owner ON activity(owner_sub,occurred_at DESC)",
      "CREATE INDEX IF NOT EXISTS idx_activity_occurred ON activity(occurred_at)",
      "CREATE INDEX IF NOT EXISTS idx_tasks_owner_parent_sort ON tasks(owner_sub,parent_id,sort_order)",
      "CREATE INDEX IF NOT EXISTS idx_tasks_completed ON tasks(completed_at)",
      "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at)",
    ] {sqlx::query(sql).execute(db).await?;}
    for sql in ["ALTER TABLE shared_tasks ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0","ALTER TABLE shared_tasks ADD COLUMN expires_at INTEGER","ALTER TABLE shared_tasks ADD COLUMN root_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE"] {let _=sqlx::query(sql).execute(db).await;}
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_shared_tasks_owner_created ON shared_tasks(owner_sub,created_at DESC)").execute(db).await?;
    for sql in ["ALTER TABLE tasks ADD COLUMN category TEXT NOT NULL DEFAULT ''", "ALTER TABLE tasks ADD COLUMN category_color TEXT NOT NULL DEFAULT '#7c8cff'", "ALTER TABLE tasks ADD COLUMN recurrence TEXT NOT NULL DEFAULT 'none'", "ALTER TABLE tasks ADD COLUMN next_reset_at INTEGER"] { let _=sqlx::query(sql).execute(db).await; }
    sqlx::query("UPDATE tasks SET category_color='' WHERE category!='' AND category_color='#7c8cff'").execute(db).await?;
    let categories:Vec<String>=sqlx::query_scalar("SELECT DISTINCT category FROM tasks WHERE category != ''").fetch_all(db).await?;
    for category in categories {let normalized=category.trim().to_lowercase();if normalized!=category {sqlx::query("UPDATE tasks SET category=? WHERE category=?").bind(normalized).bind(category).execute(db).await?;}}
    let migrated:bool=sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM app_meta WHERE key='calendar_recurrence_v1'").fetch_one(db).await?>0;
    if !migrated {
        let tasks:Vec<(String,String,i64)>=sqlx::query_as("SELECT id,recurrence,COALESCE(completed_at,created_at) FROM tasks WHERE recurrence!='none'").fetch_all(db).await?;
        let mut tx=db.begin().await?;
        for (id,recurrence,base) in tasks {sqlx::query("UPDATE tasks SET next_reset_at=? WHERE id=?").bind(recurrence_after(base,&recurrence)).bind(id).execute(&mut *tx).await?;}
        sqlx::query("INSERT INTO app_meta(key) VALUES ('calendar_recurrence_v1')").execute(&mut *tx).await?;
        tx.commit().await?;
    }
    Ok(())
}
async fn cleanup(db:&SqlitePool)->Result<()> {
    let now=Timestamp::now().as_second();
    let mut tx=db.begin().await?;
    let due:Vec<(String,String,String,String)>=sqlx::query_as("SELECT id,owner_sub,title,recurrence FROM tasks WHERE recurrence!='none' AND next_reset_at IS NOT NULL AND next_reset_at<=? ORDER BY next_reset_at,id").bind(now).fetch_all(&mut *tx).await?;
    for (id,owner,title,recurrence) in due {
        let changed=sqlx::query("WITH RECURSIVE branch(id) AS (SELECT id FROM tasks WHERE id=? AND owner_sub=? UNION SELECT t.id FROM tasks t JOIN branch b ON t.parent_id=b.id WHERE t.owner_sub=?) UPDATE tasks SET completed_at=NULL WHERE id IN (SELECT id FROM branch) AND completed_at IS NOT NULL").bind(&id).bind(&owner).bind(&owner).execute(&mut *tx).await?.rows_affected();
        if changed>0 {sqlx::query("INSERT INTO activity(owner_sub,kind,title,occurred_at) VALUES (?,'reset',?,?)").bind(&owner).bind(&title).bind(now).execute(&mut *tx).await?;}
        sqlx::query("UPDATE tasks SET next_reset_at=? WHERE id=? AND owner_sub=?").bind(recurrence_after(now,&recurrence)).bind(&id).bind(&owner).execute(&mut *tx).await?;
    }
    // Les enfants d'une tâche supprimée sont supprimés en cascade, même non cochés.
    let eligible="WITH RECURSIVE protected(id,owner_sub) AS (SELECT id,owner_sub FROM tasks WHERE recurrence!='none' UNION SELECT t.id,t.owner_sub FROM tasks t JOIN protected p ON t.parent_id=p.id AND t.owner_sub=p.owner_sub), ancestors(id,owner_sub) AS (SELECT id,owner_sub FROM tasks WHERE recurrence!='none' UNION SELECT t.parent_id,t.owner_sub FROM tasks t JOIN ancestors a ON t.id=a.id AND t.owner_sub=a.owner_sub WHERE t.parent_id IS NOT NULL), removed(id,owner_sub) AS (SELECT id,owner_sub FROM tasks WHERE completed_at IS NOT NULL AND completed_at<=? AND id NOT IN (SELECT id FROM protected UNION SELECT id FROM ancestors) UNION SELECT t.id,t.owner_sub FROM tasks t JOIN removed r ON t.parent_id=r.id AND t.owner_sub=r.owner_sub)";
    // Les deux fragments SQL sont des constantes ; les seules valeurs variables sont liées ci-dessous.
    sqlx::query(sqlx::AssertSqlSafe(format!("{eligible} INSERT INTO activity(owner_sub,kind,title,occurred_at) SELECT t.owner_sub,'deleted',t.title,? FROM tasks t JOIN removed r ON t.id=r.id AND t.owner_sub=r.owner_sub"))).bind(now-86400).bind(now).execute(&mut *tx).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("{eligible} DELETE FROM tasks WHERE id IN (SELECT id FROM removed)"))).bind(now-86400).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM activity WHERE occurred_at<?").bind(now-90*86400).execute(&mut *tx).await?;
    tx.commit().await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at<=?").bind(now).execute(db).await?;
    sqlx::query("DELETE FROM auth_flows WHERE expires_at<=?").bind(now).execute(db).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_validation_keeps_production_strict() {
        let test_url="http://192.168.1.50:3080";
        assert!(origin_allowed(test_url,test_url,true));
        assert!(origin_allowed("http://localhost:3080",test_url,true));
        assert!(origin_allowed("http://127.0.0.1:3080",test_url,true));
        assert!(!origin_allowed("http://localhost:3081",test_url,true));
        assert!(!origin_allowed("http://example.com:3080",test_url,true));
        assert!(!origin_allowed("https://example.com",test_url,true));
        assert!(!origin_allowed("http://localhost:3080","https://todo.example.com",false));
        assert!(origin_allowed("https://todo.example.com","https://todo.example.com",false));
    }

    #[tokio::test]
    async fn cleanup_preserves_and_resets_recurring_branches() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('user','User')").execute(&db).await?;
        let now=Timestamp::now().as_second();
        for (id,parent,recurrence,reset) in [("ancestor",None,"none",None),("weekly",Some("ancestor"),"weekly",Some(now+10)),("child",Some("weekly"),"none",None),("ordinary",None,"none",None)] {
            sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,recurrence,sort_order,created_at,completed_at,next_reset_at) VALUES (?,'user',?,?,?,0,?,?,?)")
                .bind(id).bind(parent).bind(id).bind(recurrence).bind(now).bind(now-90000).bind(reset).execute(&db).await?;
        }
        cleanup(&db).await?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM tasks ORDER BY id").fetch_all(&db).await?;
        assert_eq!(ids,vec!["ancestor","child","weekly"]);
        sqlx::query("UPDATE tasks SET next_reset_at=? WHERE id='weekly'").bind(now-1).execute(&db).await?;
        cleanup(&db).await?;
        let states:Vec<(String,Option<i64>)>=sqlx::query_as("SELECT id,completed_at FROM tasks ORDER BY id").fetch_all(&db).await?;
        assert_eq!(states,vec![("ancestor".to_string(),Some(now-90000)),("child".to_string(),None),("weekly".to_string(),None)]);
        let events:Vec<(String,String)>=sqlx::query_as("SELECT kind,title FROM activity WHERE owner_sub='user' ORDER BY id").fetch_all(&db).await?;
        assert_eq!(events,vec![("deleted".into(),"ordinary".into()),("reset".into(),"weekly".into())]);
        cleanup(&db).await?;
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM activity").fetch_one(&db).await?,2);
        Ok(())
    }
    #[test]
    fn calendar_boundaries_follow_paris_and_dst() {
        let paris=TimeZone::get("Europe/Paris").unwrap();
        for (from,period,expected) in [
            ("2026-03-28T20:00:00Z","daily","2026-03-28T23:00:00Z"),
            ("2026-03-29T20:00:00Z","daily","2026-03-29T22:00:00Z"),
            ("2026-10-24T20:00:00Z","daily","2026-10-24T22:00:00Z"),
            ("2026-10-25T20:00:00Z","daily","2026-10-25T23:00:00Z"),
            ("2026-03-29T20:00:00Z","weekly","2026-03-29T22:00:00Z"),
            ("2026-02-28T10:00:00Z","monthly","2026-02-28T23:00:00Z"),
            ("2026-03-31T10:00:00Z","quarterly","2026-03-31T22:00:00Z"),
            ("2026-12-31T10:00:00Z","yearly","2026-12-31T23:00:00Z"),
        ] {
            let from=from.parse::<Timestamp>().unwrap().as_second();
            let expected=expected.parse::<Timestamp>().unwrap().as_second();
            assert_eq!(recurrence_after_in(from,period,&paris),Some(expected),"{period}");
        }
    }
    #[test]
    fn calendar_handles_midnight_dst_gap() {
        let tz=TimeZone::get("America/Sao_Paulo").unwrap();
        let from="2018-11-03T20:00:00Z".parse::<Timestamp>().unwrap().as_second();
        let expected="2018-11-04T03:00:00Z".parse::<Timestamp>().unwrap().as_second();
        assert_eq!(recurrence_after_in(from,"daily",&tz),Some(expected));
    }
    #[tokio::test]
    async fn cleanup_records_cascades_and_resets_unchecked_parent() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        let now=Timestamp::now().as_second();
        for (id,owner,parent,recurrence,completed,reset) in [("r","a",None,"weekly",None,Some(now-1)),("rc","a",Some("r"),"none",Some(now-90000),None),("p","a",None,"none",Some(now-90000),None),("c","a",Some("p"),"none",None,None),("other","b",None,"none",None,None)] {
            sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,recurrence,sort_order,created_at,completed_at,next_reset_at) VALUES (?,?,?,?,?,0,?,?,?)").bind(id).bind(owner).bind(parent).bind(id).bind(recurrence).bind(now).bind(completed).bind(reset).execute(&db).await?;
        }
        cleanup(&db).await?;
        let events:Vec<(String,String,String)>=sqlx::query_as("SELECT owner_sub,kind,title FROM activity ORDER BY kind,title").fetch_all(&db).await?;
        assert_eq!(events,vec![("a".into(),"deleted".into(),"c".into()),("a".into(),"deleted".into(),"p".into()),("a".into(),"reset".into(),"r".into())]);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks WHERE id='rc' AND completed_at IS NULL").fetch_one(&db).await?,1);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks WHERE owner_sub='b'").fetch_one(&db).await?,1);
        Ok(())
    }
    #[tokio::test]
    async fn activity_and_positions_are_scoped_to_each_user() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        let token="a".repeat(43);
        sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,'a',?)").bind(digest(&token)).bind(Timestamp::now().as_second()+60).execute(&db).await?;
        sqlx::query("INSERT INTO activity(owner_sub,kind,title,occurred_at) VALUES ('a','deleted','visible',1),('b','deleted','privé',2)").execute(&db).await?;
        sqlx::query("INSERT INTO tasks(id,owner_sub,title,sort_order,created_at) VALUES ('a1','a','A1',0,0),('a2','a','A2',1,0),('b1','b','B1',0,0)").execute(&db).await?;
        let discovery=Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()};
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:discovery,issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let mut headers=HeaderMap::new();headers.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}"))?);
        let Json(events)=list_activity(State(state.clone()),headers).await.map_err(|_|anyhow::anyhow!("activity request failed"))?;
        assert_eq!(events.len(),1);assert_eq!(events[0].title,"visible");
        set_position(&state,"a","a2",0).await.map_err(|_|anyhow::anyhow!("position failed"))?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM tasks WHERE owner_sub='a' ORDER BY sort_order").fetch_all(&db).await?;
        assert_eq!(ids,vec!["a2","a1"]);
        assert!(set_position(&state,"a","b1",0).await.is_err());
        Ok(())
    }
    #[test]
    fn category_colors_are_deterministic() {
        assert_eq!(category_color(""),0);
        assert_eq!(category_color("maison"),category_color(&"MAISON".to_lowercase()));
        assert!((1..=10).contains(&category_color("\u{e9}cole")));
    }
    #[tokio::test]
    async fn category_update_is_private_and_normalized() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        sqlx::query("INSERT INTO tasks(id,owner_sub,title,description,category,sort_order,created_at) VALUES ('one','a','Titre','Texte','ancienne',0,0),('two','b','Autre','','',0,0)").execute(&db).await?;
        let token="a".repeat(43);
        sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,?,?)").bind(digest(&token)).bind("a").bind(Timestamp::now().as_second()+60).execute(&db).await?;
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()},issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let mut headers=HeaderMap::new();headers.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}"))?);headers.insert("x-todo-request",HeaderValue::from_static("1"));
        assert!(set_category(State(state.clone()),Path("two".into()),headers.clone(),Json(CategoryInput{category:"test".into()})).await.is_err());
        set_category(State(state),Path("one".into()),headers,Json(CategoryInput{category:" MAISON ".into()})).await.map_err(|_|anyhow::anyhow!("category update failed"))?;
        let result:(String,String)=sqlx::query_as("SELECT category,description FROM tasks WHERE id='one'").fetch_one(&db).await?;
        assert_eq!(result,("maison".into(),"Texte".into()));
        Ok(())
    }
    #[tokio::test]
    async fn sharing_copies_a_snapshot_for_another_user() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        let ta="a".repeat(43);let tb="b".repeat(43);
        for (token,owner) in [(&ta,"a"),(&tb,"b")] {sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,?,?)").bind(digest(token)).bind(owner).bind(Timestamp::now().as_second()+60).execute(&db).await?;}
        sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,description,category,recurrence,sort_order,created_at,completed_at) VALUES ('root','a',NULL,'Parent','Description','maison','weekly',0,0,1),('child','a','root','Enfant','','courses','none',0,0,1),('other','a',NULL,'Privé','','','none',1,0,NULL)").execute(&db).await?;
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()},issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let headers=|token:&str|{let mut h=HeaderMap::new();h.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}")).unwrap());h.insert("x-todo-request",HeaderValue::from_static("1"));h};
        assert!(create_share(State(state.clone()),Path("root".into()),headers(&tb),Json(ShareOptions::default())).await.is_err());
        let Json(link)=create_share(State(state.clone()),Path("root".into()),headers(&ta),Json(ShareOptions::default())).await.map_err(|_|anyhow::anyhow!("share failed"))?;
        let token=link["token"].as_str().unwrap().to_string();
        assert!(link["expires_at"].is_null());
        let Json(links)=list_shares(State(state.clone()),headers(&ta)).await.map_err(|_|anyhow::anyhow!("list shares failed"))?;
        assert_eq!(links.len(),1);assert_eq!(links[0]["title"],"Parent");assert_eq!(links[0]["task_count"],2);
        assert_eq!(links[0]["id"],digest(&token));assert!(links[0].get("token").is_none());
        let Json(private)=list_shares(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list shares failed"))?;
        assert!(private.is_empty());
        sqlx::query("UPDATE tasks SET title='Modifié' WHERE id='root'").execute(&db).await?;
        let Json(preview)=preview_share(State(state.clone()),Path(token.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("preview failed"))?;
        assert_eq!(preview["tasks"].as_array().unwrap().len(),2);assert_eq!(preview["tasks"][0]["title"],"Parent");assert_eq!(preview["already_imported"],false);
        assert!(import_share(State(state.clone()),Path(token.clone()),headers(&tb),Json(ShareImportInput{parent_id:Some("other".into()),new_parent_title:None})).await.is_err());
        let Json(result)=import_share(State(state.clone()),Path(token.clone()),headers(&tb),Json(ShareImportInput::default())).await.map_err(|_|anyhow::anyhow!("import failed"))?;
        let root=result["id"].as_str().unwrap();
        let copy:(String,String,String,Option<i64>,Option<i64>)=sqlx::query_as("SELECT title,category,recurrence,completed_at,next_reset_at FROM tasks WHERE id=? AND owner_sub='b'").bind(root).fetch_one(&db).await?;
        assert_eq!((&copy.0,&copy.1,&copy.2),(&"Parent".to_string(),&"maison".to_string(),&"weekly".to_string()));assert!(copy.3.is_none()&&copy.4.is_some());
        let child:(String,String)=sqlx::query_as("SELECT title,category FROM tasks WHERE owner_sub='b' AND parent_id=?").bind(root).fetch_one(&db).await?;assert_eq!(child,("Enfant".into(),"courses".into()));
        let Json(again)=preview_share(State(state.clone()),Path(token.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("preview failed"))?;assert_eq!(again["already_imported"],true);
        let Json(nested)=import_share(State(state.clone()),Path(token.clone()),headers(&tb),Json(ShareImportInput{parent_id:Some(root.into()),new_parent_title:None})).await.map_err(|_|anyhow::anyhow!("nested import failed"))?;
        let parent:Option<String>=sqlx::query_scalar("SELECT parent_id FROM tasks WHERE id=?").bind(nested["id"].as_str().unwrap()).fetch_one(&db).await?;assert_eq!(parent.as_deref(),Some(root));
        let Json(created)=import_share(State(state.clone()),Path(token.clone()),headers(&tb),Json(ShareImportInput{parent_id:None,new_parent_title:Some(" Nouveau ".into())})).await.map_err(|_|anyhow::anyhow!("new parent import failed"))?;
        let title:String=sqlx::query_scalar("SELECT p.title FROM tasks t JOIN tasks p ON p.id=t.parent_id WHERE t.id=?").bind(created["id"].as_str().unwrap()).fetch_one(&db).await?;assert_eq!(title,"Nouveau");
        assert!(revoke_share(State(state.clone()),Path(digest(&token)),headers(&tb)).await.is_err());
        revoke_share(State(state.clone()),Path(digest(&token)),headers(&ta)).await.map_err(|_|anyhow::anyhow!("revoke failed"))?;
        assert!(preview_share(State(state),Path(token),headers(&tb)).await.is_err());
        Ok(())
    }
    #[tokio::test]
    async fn live_sharing_synchronizes_a_nested_branch_until_revocation_or_expiry() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        let now=Timestamp::now().as_second();let ta="a".repeat(43);let tb="b".repeat(43);
        for (token,owner) in [(&ta,"a"),(&tb,"b")] {
            sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,?,?)").bind(digest(token)).bind(owner).bind(now+3600).execute(&db).await?;
        }
        sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,created_at) VALUES ('private-parent','a',NULL,'Privé',0),('root','a','private-parent','Cadeaux',0),('child','a','root','Un livre',0),('private-sibling','a','private-parent','Privé aussi',0)").execute(&db).await?;
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()},issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let headers=|token:&str|{let mut h=HeaderMap::new();h.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}")).unwrap());h.insert("x-todo-request",HeaderValue::from_static("1"));h};
        let Json(before)=list_tasks(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list failed"))?;
        assert!(before.is_empty());
        assert!(check_task(State(state.clone()),Path("child".into()),headers(&tb),Json(CheckInput{done:true})).await.is_err());
        let Json(link)=create_share(State(state.clone()),Path("root".into()),headers(&ta),Json(ShareOptions{duration_seconds:Some(86400),mode:"live".into()})).await.map_err(|_|anyhow::anyhow!("share failed"))?;
        assert_eq!(link["live"],true);let token=link["token"].as_str().unwrap().to_string();
        sqlx::query("UPDATE tasks SET title='Cadeaux actualisés' WHERE id='root'").execute(&db).await?;
        sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,created_at) VALUES ('new-child','a','root','Nouveau cadeau',0)").execute(&db).await?;
        let Json(preview)=preview_share(State(state.clone()),Path(token.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("preview failed"))?;
        assert_eq!(preview["live"],true);assert_eq!(preview["tasks"][0]["title"],"Cadeaux actualisés");assert_eq!(preview["tasks"].as_array().unwrap().len(),3);
        let Json(visible)=list_tasks(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list failed"))?;
        assert_eq!(visible.len(),3);assert!(visible.iter().find(|task|task.id=="root").unwrap().parent_id.is_none());
        assert_eq!(visible.iter().find(|task|task.id=="child").unwrap().parent_id.as_deref(),Some("root"));
        assert!(check_task(State(state.clone()),Path("private-parent".into()),headers(&tb),Json(CheckInput{done:true})).await.is_err());
        assert!(check_task(State(state.clone()),Path("private-sibling".into()),headers(&tb),Json(CheckInput{done:true})).await.is_err());
        assert!(set_category(State(state.clone()),Path("root".into()),headers(&tb),Json(CategoryInput{category:"autre".into()})).await.is_err());
        assert!(create_share(State(state.clone()),Path("root".into()),headers(&tb),Json(ShareOptions::default())).await.is_err());
        let _=check_task(State(state.clone()),Path("child".into()),headers(&tb),Json(CheckInput{done:true})).await.map_err(|_|anyhow::anyhow!("shared check failed"))?;
        let Json(owner)=list_tasks(State(state.clone()),headers(&ta)).await.map_err(|_|anyhow::anyhow!("list failed"))?;
        assert!(owner.iter().find(|task|task.id=="child").unwrap().completed_at.is_some());
        assert_eq!(owner.iter().find(|task|task.id=="root").unwrap().parent_id.as_deref(),Some("private-parent"));
        let _=check_task(State(state.clone()),Path("child".into()),headers(&ta),Json(CheckInput{done:false})).await.map_err(|_|anyhow::anyhow!("owner uncheck failed"))?;
        let Json(updated)=list_tasks(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list failed"))?;
        assert!(updated.iter().find(|task|task.id=="child").unwrap().completed_at.is_none());
        let Json(links)=list_shares(State(state.clone()),headers(&ta)).await.map_err(|_|anyhow::anyhow!("list shares failed"))?;assert_eq!(links[0]["live"],true);
        assert!(revoke_share(State(state.clone()),Path(digest(&token)),headers(&tb)).await.is_err());
        revoke_share(State(state.clone()),Path(digest(&token)),headers(&ta)).await.map_err(|_|anyhow::anyhow!("revoke failed"))?;
        let Json(revoked)=list_tasks(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list failed"))?;assert!(revoked.is_empty());
        assert!(check_task(State(state.clone()),Path("child".into()),headers(&tb),Json(CheckInput{done:true})).await.is_err());
        assert!(preview_share(State(state.clone()),Path(token),headers(&tb)).await.is_err());
        let Json(link)=create_share(State(state.clone()),Path("root".into()),headers(&ta),Json(ShareOptions{duration_seconds:Some(86400),mode:"live".into()})).await.map_err(|_|anyhow::anyhow!("share failed"))?;
        let token=link["token"].as_str().unwrap().to_string();
        let _=preview_share(State(state.clone()),Path(token.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("preview failed"))?;
        sqlx::query("UPDATE shared_tasks SET expires_at=?").bind(now-1).execute(&db).await?;
        let Json(expired)=list_tasks(State(state.clone()),headers(&tb)).await.map_err(|_|anyhow::anyhow!("list failed"))?;assert!(expired.is_empty());
        assert!(check_task(State(state.clone()),Path("child".into()),headers(&tb),Json(CheckInput{done:true})).await.is_err());
        assert!(preview_share(State(state),Path(token),headers(&tb)).await.is_err());
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks WHERE owner_sub='b'").fetch_one(&db).await?,0);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks WHERE owner_sub='a'").fetch_one(&db).await?,5);
        Ok(())
    }
    #[tokio::test]
    async fn position_failure_rolls_back_all_changes() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A')").execute(&db).await?;
        sqlx::query("INSERT INTO tasks(id,owner_sub,title,sort_order,created_at) VALUES ('a1','a','A1',0,0),('a2','a','A2',1,0),('a3','a','A3',2,0)").execute(&db).await?;
        sqlx::query("CREATE TRIGGER fail_reorder BEFORE UPDATE OF sort_order ON tasks WHEN OLD.id='a1' BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&db).await?;
        let discovery=Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()};
        let state=AppState{db:db.clone(),http:reqwest::Client::new(),oidc:discovery,issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()};
        assert!(set_position(&state,"a","a3",0).await.is_err());
        let values:Vec<(String,i64)>=sqlx::query_as("SELECT id,sort_order FROM tasks WHERE owner_sub='a' ORDER BY id").fetch_all(&db).await?;
        assert_eq!(values,vec![("a1".into(),0),("a2".into(),1),("a3".into(),2)]);
        Ok(())
    }
    #[tokio::test]
    async fn activity_expires_after_ninety_days() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A')").execute(&db).await?;
        let now=Timestamp::now().as_second();
        sqlx::query("INSERT INTO activity(owner_sub,kind,title,occurred_at) VALUES ('a','deleted','old',?),('a','reset','recent',?)").bind(now-91*86400).bind(now-89*86400).execute(&db).await?;
        cleanup(&db).await?;
        let titles:Vec<String>=sqlx::query_scalar("SELECT title FROM activity WHERE owner_sub='a'").fetch_all(&db).await?;
        assert_eq!(titles,vec!["recent"]);
        Ok(())
    }
    #[tokio::test]
    async fn upcoming_activity_matches_protection_cascades_and_user_scope() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A'),('b','B')").execute(&db).await?;
        let now=Timestamp::now().as_second();
        for (id,owner,parent,recurrence,completed,reset) in [
            ("ancestor","a",None,"none",Some(now-90000),None),
            ("recurring","a",Some("ancestor"),"weekly",None,Some(now+600)),
            ("nested-recurring","a",Some("recurring"),"daily",None,Some(now+300)),
            ("protected-child","a",Some("nested-recurring"),"none",Some(now-90000),None),
            ("sibling","a",Some("ancestor"),"none",Some(now-500),None),
            ("parent","a",None,"none",Some(now-90000),None),
            ("child","a",Some("parent"),"none",None,None),
            ("grandchild","a",Some("child"),"none",None,None),
            ("early-child","a",Some("parent"),"none",Some(now-90100),None),
            ("z-parent","a",None,"none",Some(now-10),None),
            ("a-child","a",Some("z-parent"),"none",Some(now-10),None),
            ("other-user","b",None,"none",Some(now-90000),None),
        ] {
            sqlx::query("INSERT INTO tasks(id,owner_sub,parent_id,title,recurrence,completed_at,next_reset_at,created_at) VALUES (?,?,?,?,?,?,?,?)")
                .bind(id).bind(owner).bind(parent).bind(id).bind(recurrence).bind(completed).bind(reset).bind(now-100000).execute(&db).await?;
        }
        let token="a".repeat(43);
        sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,'a',?)").bind(digest(&token)).bind(now+60).execute(&db).await?;
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()},issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let mut headers=HeaderMap::new();headers.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}"))?);
        assert!(list_upcoming_activity(State(state.clone()),HeaderMap::new()).await.is_err());
        let Json(plan)=list_upcoming_activity(State(state),headers).await.map_err(|_|anyhow::anyhow!("upcoming activity failed"))?;
        let events=plan["events"].as_array().unwrap();assert_eq!(events.len(),6);
        let event=|id:&str|events.iter().find(|event|event["task_id"]==id).unwrap();
        assert_eq!(event("recurring")["scheduled_at"],now+600);assert_eq!(event("recurring")["affected_count"],0);
        assert_eq!(event("nested-recurring")["affected_count"],1);
        assert_eq!(event("early-child")["affected_count"],1);assert_eq!(event("parent")["affected_count"],3);
        assert_eq!(event("z-parent")["affected_count"],2);assert_eq!(event("sibling")["affected_count"],1);
        for id in ["ancestor","protected-child","a-child","other-user"] {assert!(!events.iter().any(|event|event["task_id"]==id));}
        assert!(events.windows(2).all(|pair|pair[0]["scheduled_at"].as_i64()<=pair[1]["scheduled_at"].as_i64()));
        // La consultation ne change ni les tâches ni l'historique.
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks").fetch_one(&db).await?,12);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM activity").fetch_one(&db).await?,0);
        let planned_deletions:i64=events.iter().filter(|event|event["kind"]=="deleted"&&event["scheduled_at"].as_i64().unwrap()<=now).map(|event|event["affected_count"].as_i64().unwrap()).sum();
        cleanup(&db).await?;
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM activity WHERE owner_sub='a' AND kind='deleted'").fetch_one(&db).await?,planned_deletions);
        sqlx::query("UPDATE tasks SET next_reset_at=? WHERE id='recurring'").bind(now-1).execute(&db).await?;
        sqlx::query("UPDATE tasks SET next_reset_at=? WHERE id='nested-recurring'").bind(now-2).execute(&db).await?;
        cleanup(&db).await?;
        let resets:Vec<String>=sqlx::query_scalar("SELECT title FROM activity WHERE owner_sub='a' AND kind='reset'").fetch_all(&db).await?;
        assert_eq!(resets,vec!["nested-recurring"]);
        Ok(())
    }
    #[tokio::test]
    async fn expired_shares_cannot_be_previewed_or_imported() -> Result<()> {
        let db=SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;init_db(&db).await?;
        sqlx::query("INSERT INTO users(sub,name) VALUES ('a','A')").execute(&db).await?;
        sqlx::query("INSERT INTO tasks(id,owner_sub,title,created_at) VALUES ('task','a','Titre',0)").execute(&db).await?;
        let token="a".repeat(43);let now=Timestamp::now().as_second();
        sqlx::query("INSERT INTO sessions(token_hash,user_sub,expires_at) VALUES (?,'a',?)").bind(digest(&token)).bind(now+60).execute(&db).await?;
        let state=Arc::new(AppState{db:db.clone(),http:reqwest::Client::new(),oidc:Discovery{issuer:String::new(),authorization_endpoint:String::new(),token_endpoint:String::new(),jwks_uri:String::new()},issuer:String::new(),client_id:String::new(),client_secret:String::new(),public_url:String::new()});
        let mut headers=HeaderMap::new();headers.insert(header::COOKIE,HeaderValue::from_str(&format!("todo_session={token}"))?);headers.insert("x-todo-request",HeaderValue::from_static("1"));
        assert!(create_share(State(state.clone()),Path("task".into()),headers.clone(),Json(ShareOptions{duration_seconds:Some(-1),mode:String::new()})).await.is_err());
        let Json(link)=create_share(State(state.clone()),Path("task".into()),headers.clone(),Json(ShareOptions{duration_seconds:Some(86400),mode:String::new()})).await.map_err(|_|anyhow::anyhow!("share failed"))?;
        assert!(link["expires_at"].as_i64().unwrap()>=now+86400);
        let shared=link["token"].as_str().unwrap();
        sqlx::query("UPDATE shared_tasks SET expires_at=?").bind(now-1).execute(&db).await?;
        let error=preview_share(State(state.clone()),Path(shared.into()),headers.clone()).await.unwrap_err();
        assert_eq!(error.0,StatusCode::NOT_FOUND);
        assert!(import_share(State(state.clone()),Path(shared.into()),headers.clone(),Json(ShareImportInput::default())).await.is_err());
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM tasks").fetch_one(&db).await?,1);
        let Json(links)=list_shares(State(state),headers).await.map_err(|_|anyhow::anyhow!("list shares failed"))?;
        assert_eq!(links.len(),1);assert_eq!(links[0]["expires_at"],now-1);
        Ok(())
    }
}
#[tokio::main]
async fn main()->Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_|"info".into())).init();
    TimeZone::get(&env::var("TODO_TIMEZONE").unwrap_or_else(|_|"Europe/Paris".into())).context("TODO_TIMEZONE doit être un fuseau IANA valide")?;
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
      .route("/api/activity",get(list_activity))
      .route("/api/activity/upcoming",get(list_upcoming_activity))
      .route("/api/tasks/{id}",axum::routing::patch(edit_task).delete(delete_task))
      .route("/api/tasks/{id}/category",axum::routing::patch(set_category))
      .route("/api/tasks/{id}/share",post(create_share))
      .route("/api/shares/{token}",get(preview_share).post(import_share))
      .route("/api/shares",get(list_shares))
      .route("/api/shares/manage/{id}",axum::routing::delete(revoke_share))
      .route("/api/tasks/{id}/check",post(check_task)).route("/api/tasks/{id}/move",post(move_task)).route("/api/tasks/{id}/position",post(position_task))
      .with_state(s);
    let listener=tokio::net::TcpListener::bind(("0.0.0.0",port)).await?;
    info!("Todo démarre sur 0.0.0.0:{port}");
    axum::serve(listener,app).await?;
    Ok(())
}
