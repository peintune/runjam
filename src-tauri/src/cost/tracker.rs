use rusqlite::{Connection, params};
use serde::Serialize;
use chrono::Local;

#[derive(Debug, Serialize)]
pub struct CostSummary {
    pub today_tokens: i64,
    pub week_tokens: i64,
    pub month_tokens: i64,
    pub total_tokens: i64,
    pub today_cost: f64,
    pub week_cost: f64,
    pub month_cost: f64,
    pub total_cost: f64,
    pub today_cached_tokens: i64,
    pub week_cached_tokens: i64,
    pub month_cached_tokens: i64,
    pub total_cached_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct AgentCost {
    pub agent_id: String,
    pub agent_name: String,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub sessions: i64,
    pub cached_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyCost {
    pub date: String,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub cached_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct SessionCost {
    pub session_id: String,
    pub agent_name: String,
    pub directory: String,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub created_at: String,
    pub message_count: i64,
    pub cached_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct DirectoryCost {
    pub directory: String,
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub sessions: i64,
    pub cached_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct SessionTokenUsage {
    pub session_id: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
}

/// Record a token usage entry for a session.
/// Called from the ACP client when usage_update events arrive.
pub fn record_usage(
    conn: &Connection,
    session_id: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cost_estimate: f64,
    cached_tokens: i64,
) {
    let id = format!("tu_{}", Local::now().timestamp_nanos());
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO token_usage (id, session_id, model, input_tokens, output_tokens, cost_estimate, cached_tokens, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, session_id, model, input_tokens, output_tokens, cost_estimate, cached_tokens, now],
    ).ok();
}

/// Get overall cost summary (today / this week / this month / all time).
pub fn get_cost_summary(conn: &Connection) -> CostSummary {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let week_ago = (Local::now() - chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
    let month_ago = (Local::now() - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();

    let query = |since: &str| -> (i64, f64, i64) {
        conn.query_row(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0), COALESCE(SUM(cost_estimate), 0.0), COALESCE(SUM(cached_tokens), 0)
             FROM token_usage WHERE recorded_at >= ?1",
            params![since],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap_or((0, 0.0, 0))
    };

    let (today_tokens, today_cost, today_cached) = query(&today);
    let (week_tokens, week_cost, week_cached) = query(&week_ago);
    let (month_tokens, month_cost, month_cached) = query(&month_ago);

    let (total_tokens, total_cost, total_cached): (i64, f64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens), 0), COALESCE(SUM(cost_estimate), 0.0), COALESCE(SUM(cached_tokens), 0) FROM token_usage",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap_or((0, 0.0, 0));

    CostSummary {
        today_tokens, week_tokens, month_tokens, total_tokens,
        today_cost, week_cost, month_cost, total_cost,
        today_cached_tokens: today_cached,
        week_cached_tokens: week_cached,
        month_cached_tokens: month_cached,
        total_cached_tokens: total_cached,
    }
}

/// Get cost breakdown by agent (JOIN token_usage → sessions for agent info).
pub fn get_cost_by_agent(conn: &Connection) -> Vec<AgentCost> {
    let mut stmt = conn.prepare(
        "SELECT s.cli, s.cli_display_name,
                COALESCE(SUM(tu.input_tokens + tu.output_tokens), 0),
                COALESCE(SUM(tu.input_tokens), 0),
                COALESCE(SUM(tu.output_tokens), 0),
                COALESCE(SUM(tu.cost_estimate), 0.0),
                COUNT(DISTINCT tu.session_id),
                COALESCE(SUM(tu.cached_tokens), 0)
         FROM token_usage tu
         JOIN sessions s ON s.id = tu.session_id
         GROUP BY s.cli
         ORDER BY SUM(tu.input_tokens + tu.output_tokens) DESC"
    ).unwrap();

    let results = stmt.query_map([], |row| {
        Ok(AgentCost {
            agent_id: row.get(0)?,
            agent_name: row.get(1)?,
            total_tokens: row.get(2)?,
            input_tokens: row.get(3)?,
            output_tokens: row.get(4)?,
            cost: row.get(5)?,
            sessions: row.get(6)?,
            cached_tokens: row.get(7)?,
        })
    }).unwrap();

    results.filter_map(|r| r.ok()).collect()
}

/// Get daily cost trend for the last N days.
pub fn get_cost_by_day(conn: &Connection, days: i32) -> Vec<DailyCost> {
    let since = (Local::now() - chrono::Duration::days(days as i64))
        .format("%Y-%m-%d").to_string();

    let mut stmt = conn.prepare(
        "SELECT DATE(recorded_at) as day,
                COALESCE(SUM(input_tokens + output_tokens), 0),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cost_estimate), 0.0),
                COALESCE(SUM(cached_tokens), 0)
         FROM token_usage
         WHERE recorded_at >= ?1
         GROUP BY day
         ORDER BY day ASC"
    ).unwrap();

    let results = stmt.query_map(params![since], |row| {
        Ok(DailyCost {
            date: row.get(0)?,
            total_tokens: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            cost: row.get(4)?,
            cached_tokens: row.get(5)?,
        })
    }).unwrap();

    results.filter_map(|r| r.ok()).collect()
}

/// Get per-session cost breakdown (top N sessions).
pub fn get_cost_by_session(conn: &Connection, limit: i64) -> Vec<SessionCost> {
    let mut stmt = conn.prepare(
        "SELECT tu.session_id,
                s.cli_display_name,
                COALESCE(s.directory, ''),
                COALESCE(SUM(tu.input_tokens + tu.output_tokens), 0),
                COALESCE(SUM(tu.input_tokens), 0),
                COALESCE(SUM(tu.output_tokens), 0),
                COALESCE(SUM(tu.cost_estimate), 0.0),
                s.created_at,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = tu.session_id) as msg_count,
                COALESCE(SUM(tu.cached_tokens), 0)
         FROM token_usage tu
         JOIN sessions s ON s.id = tu.session_id
         GROUP BY tu.session_id
         ORDER BY SUM(tu.input_tokens + tu.output_tokens) DESC
         LIMIT ?1"
    ).unwrap();

    let results = stmt.query_map(params![limit], |row| {
        Ok(SessionCost {
            session_id: row.get(0)?,
            agent_name: row.get(1)?,
            directory: row.get(2)?,
            total_tokens: row.get(3)?,
            input_tokens: row.get(4)?,
            output_tokens: row.get(5)?,
            cost: row.get(6)?,
            created_at: row.get(7)?,
            message_count: row.get(8)?,
            cached_tokens: row.get(9)?,
        })
    }).unwrap();

    results.filter_map(|r| r.ok()).collect()
}

/// Get per-directory (project) cost breakdown.
pub fn get_cost_by_directory(conn: &Connection) -> Vec<DirectoryCost> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(s.directory, 'Unknown'),
                COALESCE(SUM(tu.input_tokens + tu.output_tokens), 0),
                COALESCE(SUM(tu.input_tokens), 0),
                COALESCE(SUM(tu.output_tokens), 0),
                COALESCE(SUM(tu.cost_estimate), 0.0),
                COUNT(DISTINCT tu.session_id),
                COALESCE(SUM(tu.cached_tokens), 0)
         FROM token_usage tu
         JOIN sessions s ON s.id = tu.session_id
         GROUP BY s.directory
         ORDER BY SUM(tu.input_tokens + tu.output_tokens) DESC"
    ).unwrap();

    let results = stmt.query_map([], |row| {
        Ok(DirectoryCost {
            directory: row.get(0)?,
            total_tokens: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            cost: row.get(4)?,
            sessions: row.get(5)?,
            cached_tokens: row.get(6)?,
        })
    }).unwrap();

    results.filter_map(|r| r.ok()).collect()
}
