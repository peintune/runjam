use crate::cost::tracker;
use crate::db::connection::Database;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn get_cost_summary(db_state: State<'_, Mutex<Database>>) -> tracker::CostSummary {
    let db = db_state.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    tracker::get_cost_summary(&conn)
}

#[tauri::command]
pub fn get_cost_by_agent(db_state: State<'_, Mutex<Database>>) -> Vec<tracker::AgentCost> {
    let db = db_state.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    tracker::get_cost_by_agent(&conn)
}

#[tauri::command]
pub fn get_cost_by_day(db_state: State<'_, Mutex<Database>>, days: i32) -> Vec<tracker::DailyCost> {
    let db = db_state.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    tracker::get_cost_by_day(&conn, days)
}

#[tauri::command]
pub fn get_cost_by_session(db_state: State<'_, Mutex<Database>>, limit: i64) -> Vec<tracker::SessionCost> {
    let db = db_state.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    tracker::get_cost_by_session(&conn, limit)
}

#[tauri::command]
pub fn get_cost_by_directory(db_state: State<'_, Mutex<Database>>) -> Vec<tracker::DirectoryCost> {
    let db = db_state.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    tracker::get_cost_by_directory(&conn)
}
