//! Hosted account boundary: only this native module can read the platform
//! session. Model/CLI configuration contains a public marker, never a session.
pub const MODEL_ID: &str = "runjam-builtin";
pub const CHAT_URL: &str = "https://supabase-proxy.runjam.app/functions/v1/platform/runjam/ai/chat/completions";
use crate::models_config::ModelEntry;
use serde_json::{json,Value};
use std::sync::{Mutex,OnceLock};
use std::time::{Duration,Instant};
use tauri_plugin_opener::OpenerExt;
const BASE: &str = "https://supabase-proxy.runjam.app/functions/v1/platform/runjam";
const SERVICE: &str = "com.runjam.platform";
#[derive(Clone)]
struct PendingCheckout {order_id:String,url:String,owner:String,created:Instant}
static CHECKOUT: Mutex<Option<PendingCheckout>> = Mutex::new(None);
static SESSION: OnceLock<Mutex<Option<String>>> = OnceLock::new();
fn session() -> &'static Mutex<Option<String>> { SESSION.get_or_init(||Mutex::new(None)) }
fn credential() -> Result<keyring::Entry,String> {
    keyring::Entry::new(SERVICE,"session").map_err(|_|"系统凭据库暂不可用，请解锁后重试。".into())
}
fn token() -> Result<Option<String>,String> {
    let mut saved=session().lock().map_err(|_|"账号状态暂不可用")?;
    if let Some(value)=saved.as_ref(){ return Ok(if value.is_empty(){None}else{Some(value.clone())}); }
    let value=match credential()?.get_password(){Ok(v)=>v,Err(keyring::Error::NoEntry)=>String::new(),Err(_)=>return Err("无法读取系统凭据，请解锁后重试。".into())};
    let result=if value.is_empty(){None}else{Some(value.clone())};*saved=Some(value);Ok(result)
}
fn save_token(value:&str)->Result<(),String>{
    credential()?.set_password(value).map_err(|_|"无法保存登录状态，请解锁系统凭据库后重试。")?;
    *session().lock().map_err(|_|"账号状态暂不可用")?=Some(value.to_string());Ok(())
}
fn forget_token()->Result<(),String>{
    *session().lock().map_err(|_|"账号状态暂不可用")?=Some(String::new());
    match credential()?.delete_credential(){Ok(())|Err(keyring::Error::NoEntry)=>Ok(()),Err(_)=>Err("无法清除系统登录状态，请解锁凭据库后再次退出。".into())}
}
pub fn builtin_model()->ModelEntry {
    ModelEntry {id:MODEL_ID.into(),name:MODEL_ID.into(),alias:"RunJam 内置 AI".into(),provider:"runjam".into(),
        provider_name:"RunJam".into(),provider_icon:String::new(),api_base:format!("{BASE}/ai"),api_key:"runjam-managed".into(),
        protocol:"openai_chat".into(),context_window:128000,support_reasoning:false,support_tools:true,
        tags:vec!["builtin".into()],use_proxy:true,force_reasoning_none:false}
}
pub fn normalize_models(mut models:Vec<ModelEntry>)->Vec<ModelEntry>{
    models.retain(|m|m.id!=MODEL_ID&&m.provider!="runjam");models.push(builtin_model());models
}
pub fn ensure_model(conn:&rusqlite::Connection)->Result<(),String>{
    let m=builtin_model();
    conn.execute("INSERT OR IGNORE INTO models(id,name,alias,provider,provider_name,provider_icon,api_base,api_key,protocol,context_window,support_reasoning,tags,force_reasoning_none) VALUES(?1,?2,?3,'runjam','RunJam','',?4,'runjam-managed','openai_chat',128000,0,'[\"builtin\"]',0)",
        rusqlite::params![m.id,m.name,m.alias,m.api_base]).map_err(|_|"内置模型初始化失败")?;
    Ok(())
}
pub fn prepare_request(url:&str,body:&str,value:&str)->Result<(String,String),String>{
    if url!=CHAT_URL{return Err("内置 AI 地址无效。".into());}
    if value.len()<20||value.len()>200||!value.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'||b==b'-'){return Err("请在设置的账号与余额中登录后使用内置 AI。".into());}
    let mut body:Value=serde_json::from_str(body).map_err(|_|"内置 AI 请求无效")?;
    let obj=body.as_object_mut().ok_or("内置 AI 请求无效")?;
    obj.insert("model".into(),json!(MODEL_ID));
    obj.insert("request_id".into(),json!(uuid::Uuid::new_v4().to_string()));
    Ok((value.to_string(),body.to_string()))
}
pub fn authorize_chat(body:&str)->Result<(String,String),String>{
    let value=token()?.ok_or("请在设置的账号与余额中登录后使用内置 AI。")?;
    prepare_request(CHAT_URL,body,&value)
}
fn api(path:&str,body:Option<Value>,auth:bool)->Result<Value,String>{
    let saved=if auth{Some(token()?.ok_or("请先登录账号。")?)}else{None};
    let agent=ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).redirects(0).build();
    let url=format!("{BASE}{path}");
    let mut req=if body.is_some(){agent.post(&url)}else{agent.get(&url)};
    if let Some(value)=&saved{req=req.set("Authorization",&format!("Bearer {value}"));}
    let response=if let Some(body)=body{req.set("Content-Type","application/json").send_json(body)}else{req.call()};
    match response {
        Ok(r)=>r.into_json().map_err(|_|"账号服务响应无效，请稍后重试。".into()),
        Err(ureq::Error::Status(status,r))=>{
            if status==401{if let Some(value)=saved{if token().ok().flatten().as_deref()==Some(value.as_str()){let _=forget_token();}}}
            let value:Value=r.into_json().unwrap_or_default();
            Err(value.get("error").and_then(Value::as_str).filter(|m|m.chars().count()<180).unwrap_or("账号服务暂不可用，请稍后重试。").into())
        },Err(_)=>Err("无法连接账号服务，请检查网络后重试。".into())
    }
}
fn status()->Result<Value,String>{
    let catalog=api("/catalog",None,false)?;
    let account=if token()?.is_some(){Some(api("/account",None,true)?)}else{None};
    Ok(json!({"catalog":catalog,"account":account}))
}
async fn blocking<F>(f:F)->Result<Value,String>where F:FnOnce()->Result<Value,String>+Send+'static{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_|"账号服务暂不可用。".to_string())?
}
#[tauri::command]
pub async fn platform_status()->Result<Value,String>{blocking(status).await}
#[tauri::command]
pub async fn platform_send_code(email:String)->Result<Value,String>{blocking(move||api("/auth/code",Some(json!({"email":email.trim()})),false)).await}
#[tauri::command]
pub async fn platform_login(email:String,code:String)->Result<Value,String>{blocking(move||{
    let result=api("/auth/verify",Some(json!({"email":email.trim(),"code":code.trim()})),false)?;
    let value=result["token"].as_str().ok_or("登录响应无效，请重试。")?;
    if let Err(e)=save_token(value){return Err(e);}
    Ok(json!({"catalog":result["account"]["catalog"],"account":result["account"]}))
}).await}
#[tauri::command]
pub async fn platform_logout()->Result<Value,String>{blocking(||{
    let _=api("/auth/logout",Some(json!({})),true);forget_token()?;Ok(json!({"ok":true}))
}).await}
#[tauri::command]
pub async fn platform_ledger()->Result<Value,String>{blocking(||api("/ledger",None,true)).await}
#[tauri::command]
pub async fn platform_order(order_id:String)->Result<Value,String>{blocking(move||{
    uuid::Uuid::parse_str(&order_id).map_err(|_|"订单标识无效。")?;
    api(&format!("/orders/status?orderId={order_id}"),None,true)
}).await}
#[tauri::command]
pub async fn platform_checkout(app:tauri::AppHandle)->Result<Value,String>{blocking(move||{
    let owner=token()?.ok_or("请先登录账号。")?;
    let cached=CHECKOUT.lock().map_err(|_|"收银台状态暂不可用")?.clone();
    if let Some(cached)=cached.filter(|p|p.owner==owner&&p.created.elapsed()<Duration::from_secs(1500)){
        if api(&format!("/orders/status?orderId={}",cached.order_id),None,true)?["status"]=="pending"{
            app.opener().open_url(&cached.url,None::<&str>).map_err(|_|"无法打开浏览器，请稍后重试。")?;
            return Ok(json!({"orderId":cached.order_id}));
        }
    }
    let checkout=api("/checkout",Some(json!({"sku":"ai10"})),true)?;
    let raw=checkout["checkoutUrl"].as_str().ok_or("收银台暂时无法打开。")?;
    let url=url::Url::parse(raw).map_err(|_|"收银台地址无效。")?;
    if url.scheme()!="https"||url.host_str()!=Some("pancake.waffo.ai")||!url.username().is_empty()||url.password().is_some()||url.port().is_some(){return Err("收银台地址无效。".into());}
    let order_id=checkout["orderId"].as_str().ok_or("订单标识无效")?.to_string();
    *CHECKOUT.lock().map_err(|_|"收银台状态暂不可用")?=Some(PendingCheckout{order_id,url:raw.into(),owner,created:Instant::now()});
    app.opener().open_url(raw,None::<&str>).map_err(|_|"无法打开浏览器，请稍后重试。")?;
    Ok(json!({"orderId":checkout["orderId"]}))
}).await}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_model_is_canonical_and_contains_no_session() {
        let model=builtin_model();
        assert_eq!(model.id, MODEL_ID);
        assert_eq!(model.protocol,"openai_chat");
        assert_eq!(model.api_key,"runjam-managed");
        assert_eq!(format!("{}/chat/completions",model.api_base), CHAT_URL);
    }
    #[test]
    fn credential_injection_is_bound_to_exact_approved_endpoint() {
        let token="a".repeat(43);
        let prepared=prepare_request(CHAT_URL,"{\"messages\":[]}",&token).unwrap();
        assert_eq!(prepared.0,token);
        let body:serde_json::Value=serde_json::from_str(&prepared.1).unwrap();
        assert!(body["request_id"].as_str().is_some());
        for url in ["https://evil.example/chat/completions","https://supabase-proxy.runjam.app.evil.example/functions/v1/platform/runjam/ai/chat/completions","http://supabase-proxy.runjam.app/functions/v1/platform/runjam/ai/chat/completions"] {
            assert!(prepare_request(url,"{}",&token).is_err());
        }
    }
}
