use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grader {
    NormalizedExactMatch,
    AcceptedAliases,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub ground_truth: String,
    pub source_url: String,
    pub grader: Grader,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub format_hint: String,
    #[serde(default)]
    pub search_required: bool,
}
pub fn normalize(s: &str) -> String {
    s.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn grade(q: &Question, answer: &str) -> bool {
    let answer = normalize(answer);
    if answer.is_empty() {
        return false;
    }
    answer == normalize(&q.ground_truth)
        || matches!(q.grader, Grader::AcceptedAliases)
            && q.aliases.iter().any(|a| normalize(a) == answer)
}
#[derive(Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub question: String,
    pub ground_truth: String,
    pub correct: bool,
    pub answer_matches: bool,
    pub search_requirement_met: bool,
    pub result: Value,
}
#[derive(Default, Debug, Serialize)]
pub struct Totals {
    pub correct: usize,
    pub questions: usize,
    pub steps: u64,
    pub searches: u64,
    pub fetches: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub wall_ms: u64,
    pub live_tavily_calls: u64,
    pub live_tavily_calls_total: u64,
}
pub fn metric(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
pub fn totals(rows: &[Row]) -> Totals {
    let mut t = Totals::default();
    for r in rows {
        t.questions += 1;
        t.correct += usize::from(r.correct);
        t.steps += metric(&r.result, "steps");
        t.searches += metric(&r.result, "searches");
        t.fetches += metric(&r.result, "fetches");
        t.prompt_tokens += metric(&r.result, "prompt_tokens");
        t.completion_tokens += metric(&r.result, "completion_tokens");
        t.wall_ms += metric(&r.result, "wall_ms");
        t.live_tavily_calls += metric(&r.result, "live_tavily_calls");
        t.live_tavily_calls_total = t
            .live_tavily_calls_total
            .max(metric(&r.result, "live_tavily_calls_total"));
    }
    t
}
pub fn markdown(rows: &[Row]) -> String {
    let mut text = String::from("| Question | Correct | Status | Steps | Searches | Fetches | Prompt tokens | Completion tokens | Wall ms | Live Tavily |\n|---|---:|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for r in rows {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.id.replace('|', "/"),
            r.correct,
            r.result["status"].as_str().unwrap_or("unknown"),
            metric(&r.result, "steps"),
            metric(&r.result, "searches"),
            metric(&r.result, "fetches"),
            metric(&r.result, "prompt_tokens"),
            metric(&r.result, "completion_tokens"),
            metric(&r.result, "wall_ms"),
            metric(&r.result, "live_tavily_calls")
        ));
    }
    let t = totals(rows);
    text.push_str(&format!("| **Total** | **{}/{}** | | {} | {} | {} | {} | {} | {} | {} |\n\nPersistent live Tavily calls: {}.\n",t.correct,t.questions,t.steps,t.searches,t.fetches,t.prompt_tokens,t.completion_tokens,t.wall_ms,t.live_tavily_calls,t.live_tavily_calls_total));
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_and_alias_grading_does_not_accept_substrings() {
        let q: Question = serde_json::from_value(serde_json::json!({"id":"q","question":"Who?","ground_truth":"María Corina Machado","source_url":"https://example.org","grader":"accepted_aliases","aliases":["Machado"]})).unwrap();
        assert!(grade(&q, " **MARIA CORINA MACHADO.** "));
        assert!(grade(&q, "Machado"));
        assert!(!grade(&q, "It might be Machado or somebody else"));
        assert!(!grade(&q, ""));
        assert_eq!(
            normalize("Kuala Lumpur, 6–12 March 2027"),
            normalize("Kuala Lumpur, 6-12 March 2027")
        );
    }
}

pub mod loop_mode;
