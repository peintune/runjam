// Cost commands — stub for future use
use serde::Serialize;

#[derive(Serialize)]
pub struct CostSummary {
    pub today: f64,
    pub week: f64,
    pub total: f64,
    pub total_tokens: i64,
}

#[tauri::command]
pub fn get_cost_summary() -> CostSummary {
    CostSummary {
        today: 0.0,
        week: 0.0,
        total: 0.0,
        total_tokens: 0,
    }
}
