//! Teto da execução e estimado × real (`src/lib/moirai.ts:461-540`, `src/panel/moirai.ts:73-136`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hades_core::fsx::{list_dir, read_text};
use hades_core::jsfmt::to_fixed;
use hades_core::registry::plan_sha;
use hades_core::work::read_json;
use regex::Regex;
use serde_json::{Map, Number, Value};

use crate::jsnum::js_number;
use crate::plan::{EstimateRow, estimate_of_unit, parse_section};
use crate::real::{CostCache, current_cost};
use crate::transcripts::parse_ts_ms;

/// `DEFAULT_WARN_PCT` (`src/lib/moirai.ts:463`).
pub const DEFAULT_WARN_PCT: f64 = 100.0;
/// `DEFAULT_ASK_PCT` (`src/lib/moirai.ts:464`).
pub const DEFAULT_ASK_PCT: f64 = 150.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetLevel {
    Ok,
    Warn,
    Ask,
}

impl BudgetLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetLevel::Ok => "ok",
            BudgetLevel::Warn => "warn",
            BudgetLevel::Ask => "ask",
        }
    }
}

/// `Budget` (`src/lib/moirai.ts:468`); `pct` é `None` sem estimativa utilizável.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub pct: Option<f64>,
    pub warn: bool,
    pub ask: bool,
    pub level: BudgetLevel,
}

/// `budgetOf` (`src/lib/moirai.ts:478`): o custo real contra o p75 da estimativa. Sem p75 (ausente ou zero) nada passa
/// de limiar.
pub fn budget_of(cost_usd: f64, p75: Option<f64>, warn_pct: f64, ask_pct: f64) -> Budget {
    let Some(p75) = p75.filter(|p| *p > 0.0) else {
        return Budget {
            pct: None,
            warn: false,
            ask: false,
            level: BudgetLevel::Ok,
        };
    };
    let pct = cost_usd / p75 * 100.0;
    let warn = pct >= warn_pct;
    let ask = pct >= ask_pct;
    let level = if ask {
        BudgetLevel::Ask
    } else if warn {
        BudgetLevel::Warn
    } else {
        BudgetLevel::Ok
    };
    Budget {
        pct: Some(pct),
        warn,
        ask,
        level,
    }
}

/// `nextQuestionNumber` (`src/lib/moirai.ts:512`): próximo número de pergunta de um `questions.md` (cabeçalhos
/// `## Q<n>`).
pub fn next_question_number(questions_text: &str) -> u64 {
    let re = Regex::new(r"(?m)^## Q(\d+)").expect("regex fixa");
    let max = re
        .captures_iter(questions_text)
        .map(|c| c[1].parse::<u64>().unwrap_or(u64::MAX))
        .max()
        .unwrap_or(0);
    max.saturating_add(1)
}

/// `BudgetQuestionInput` (`src/lib/moirai.ts:518`).
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetQuestionInput {
    pub number: u64,
    pub unit: String,
    pub phase: String,
    pub cost_usd: f64,
    pub p75: f64,
    pub pct: f64,
    pub ask_pct: f64,
}

/// Número como o `${n}` do JS: inteiro sem `.0`.
fn js_num(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

fn phase_or_dash(phase: &str) -> &str {
    if phase.is_empty() { "—" } else { phase }
}

/// `hasBudgetQuestion` (`src/lib/moirai.ts:529`): pergunta do limiar já gravada para a fase (aberta ou respondida); a
/// resposta do dono vale para o limiar, não se repete.
pub fn has_budget_question(questions_text: &str, ask_pct: f64, phase: &str) -> bool {
    let title = format!("custo passou de {}% do estimado", js_num(ask_pct));
    let marker = format!("(fase {})", phase_or_dash(phase));
    let head = Regex::new(r"^## Q\d+").expect("regex fixa");
    let mut blocks: Vec<String> = Vec::new();
    for line in questions_text.split_inclusive('\n') {
        if head.is_match(line) || blocks.is_empty() {
            blocks.push(String::new());
        }
        if let Some(last) = blocks.last_mut() {
            last.push_str(line);
        }
    }
    blocks.iter().any(|block| {
        block.starts_with("## Q")
            && block.split('\n').next().is_some_and(|l| l.contains(&title))
            && block.contains(&marker)
    })
}

/// `budgetQuestion` (`src/lib/moirai.ts:536`): pergunta ao dono quando o custo passa do limiar de pergunta; respondida
/// pelo `hades agent answer` de sempre.
pub fn budget_question(q: &BudgetQuestionInput) -> String {
    [
        format!("## Q{} — custo passou de {}% do estimado", q.number, js_num(q.ask_pct)),
        String::new(),
        format!(
            "Contexto: {} (fase {}) já gastou US$ {}, {}% do p75 da estimativa do plano (US$ {}). O agente segue trabalhando até a resposta.",
            q.unit,
            phase_or_dash(&q.phase),
            to_fixed(q.cost_usd, 2),
            js_num(q.pct.floor()),
            to_fixed(q.p75, 2)
        ),
        String::new(),
        "Alternativas:".to_string(),
        "1. Seguir: o escopo é esse mesmo e o custo vale.".to_string(),
        "2. Reestimar: o plano subestimou; rode `hades moirai estimate <plano> --write` e aprove de novo.".to_string(),
        "3. Estacionar: parar a unidade e rever o escopo.".to_string(),
        String::new(),
        "Recomendação: se o diff está dentro do escopo do brief, seguir; se o escopo cresceu, reestimar; se o custo vem de retrabalho, estacionar.".to_string(),
        String::new(),
    ]
    .join("\n")
}

/// Código estável do erro; o texto não é contrato (`MOIRAI_ERRORS`, `src/panel/moirai.ts:15`).
pub const ERR_INVALID_UNIT: &str = "invalid_unit";
pub const ERR_UNKNOWN_UNIT: &str = "unknown_unit";

/// `MoiraiError` (`src/panel/moirai.ts:17`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoiraiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

/// `Registry` do painel: `slug` e `<raiz>/<slug>/orchestration` (`src/panel/argos-graph.ts:66`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    pub slug: String,
    pub orch: PathBuf,
}

/// `MoiraiDeps` (`src/panel/moirai.ts:29`). `now_ms` é o relógio em ms desde a época.
pub struct MoiraiDeps {
    pub registries: Box<dyn Fn() -> Vec<Registry> + Send>,
    pub config: Box<dyn Fn() -> HashMap<String, String> + Send>,
    pub projects_dir: PathBuf,
    pub now_ms: Box<dyn Fn() -> f64 + Send>,
}

/// Rotas `GET /api/moirai/plan` e `/unit/:nome`, só leitura. O nome só chega aos transcripts depois de achado por
/// igualdade entre os registros dos agentes: nunca vira caminho (`createMoiraiRoutes`, `src/panel/moirai.ts:73`).
pub struct MoiraiRoutes {
    deps: MoiraiDeps,
    caches: HashMap<String, CostCache>,
}

struct Agent {
    name: String,
    data: Map<String, Value>,
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn pct_setting(raw: Option<&String>, fallback: f64) -> f64 {
    match raw.and_then(|r| js_number(r)) {
        Some(v) if v.fract() == 0.0 && v > 0.0 && v.is_finite() => v,
        _ => fallback,
    }
}

fn agents_of(registry: &Registry) -> Vec<Agent> {
    let dir = registry.orch.join("agents");
    list_dir(&dir)
        .into_iter()
        .filter(|f| f.ends_with(".json"))
        .filter_map(|file| {
            let data = read_json(&dir.join(file))?;
            let name = text(data.get("name"));
            (!name.is_empty()).then_some(Agent { name, data })
        })
        .collect()
}

fn unit_name_ok(name: &str) -> bool {
    let bytes = name.as_bytes();
    let first_ok = bytes.first().is_some_and(u8::is_ascii_alphanumeric);
    first_ok
        && bytes.len() <= 100
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `Number` do JS como nó JSON: inteiro sem `.0`, `NaN`/infinito viram `null`.
fn json_num(n: f64) -> Value {
    if !n.is_finite() {
        return Value::Null;
    }
    if n.fract() == 0.0 && n.abs() < 9e15 {
        return Value::Number(Number::from(n as i64));
    }
    Number::from_f64(n).map_or(Value::Null, Value::Number)
}

fn opt_num(n: Option<f64>) -> Value {
    n.map_or(Value::Null, json_num)
}

fn estimate_json(e: &EstimateRow) -> Value {
    let mut m = Map::new();
    m.insert("unit".into(), Value::String(e.unit.clone()));
    m.insert("costP25".into(), opt_num(e.cost_p25));
    m.insert("costMedian".into(), opt_num(e.cost_median));
    m.insert("costP75".into(), opt_num(e.cost_p75));
    m.insert("wallP25".into(), opt_num(e.wall_p25));
    m.insert("wallMedian".into(), opt_num(e.wall_median));
    m.insert("wallP75".into(), opt_num(e.wall_p75));
    m.insert("gateMedian".into(), opt_num(e.gate_median));
    m.insert("sample".into(), json_num(e.sample));
    m.insert("confidence".into(), Value::String(e.confidence.clone()));
    m.insert("basis".into(), Value::String(e.basis.clone()));
    Value::Object(m)
}

fn non_empty(s: String) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        Value::String(s)
    }
}

/// `seconds` (`src/lib/moirai.ts:150`).
fn seconds(from: &str, to_ms: f64) -> Option<f64> {
    let a = parse_ts_ms(from);
    (!a.is_nan() && !to_ms.is_nan() && to_ms >= a).then(|| (to_ms - a) / 1000.0)
}

struct Actual {
    cost_usd: f64,
    json: Value,
}

impl MoiraiRoutes {
    pub fn new(deps: MoiraiDeps) -> Self {
        Self {
            deps,
            caches: HashMap::new(),
        }
    }

    fn limits(&self) -> (f64, f64) {
        let config = (self.deps.config)();
        (
            pct_setting(config.get("HADES_MOIRAI_WARN_PCT"), DEFAULT_WARN_PCT),
            pct_setting(config.get("HADES_MOIRAI_ASK_PCT"), DEFAULT_ASK_PCT),
        )
    }

    fn real(&mut self, registry: &Registry, agent: &Agent) -> Actual {
        let key = format!("{}/{}", registry.slug, agent.name);
        let cache = self.caches.entry(key).or_default();
        let worktree = text(agent.data.get("worktree"));
        let cost_usd = current_cost(&worktree, cache, &self.deps.projects_dir).cost_usd;
        let since = match text(agent.data.get("createdAt")) {
            s if s.is_empty() => text(agent.data.get("adoptedAt")),
            s => s,
        };
        let until = [agent.data.get("doneAt"), agent.data.get("archivedAt")]
            .into_iter()
            .map(text)
            .find(|s| !s.is_empty());
        let until_ms = until.map_or_else(|| (self.deps.now_ms)(), |u| parse_ts_ms(&u));
        let wall_s = if since.is_empty() {
            None
        } else {
            seconds(&since, until_ms)
        };
        let phase = match text(agent.data.get("phase")) {
            s if s.is_empty() => text(agent.data.get("role")),
            s => s,
        };
        let mut m = Map::new();
        m.insert("costUsd".into(), json_num(cost_usd));
        m.insert("wallMin".into(), opt_num(wall_s.map(|s| s / 60.0)));
        m.insert("state".into(), non_empty(text(agent.data.get("state"))));
        m.insert("phase".into(), non_empty(phase));
        Actual {
            cost_usd,
            json: Value::Object(m),
        }
    }

    fn row(
        &mut self,
        registry: &Registry,
        estimate: Option<&EstimateRow>,
        agent: Option<&Agent>,
    ) -> Map<String, Value> {
        let (warn_pct, ask_pct) = self.limits();
        let actual = agent.map(|a| self.real(registry, a));
        let budget = match (&actual, estimate) {
            (Some(a), Some(e)) => Some(budget_of(a.cost_usd, e.cost_p75, warn_pct, ask_pct)),
            _ => None,
        };
        let unit = estimate
            .map(|e| e.unit.clone())
            .or_else(|| agent.map(|a| a.name.clone()))
            .unwrap_or_default();
        let mut m = Map::new();
        m.insert("unit".into(), Value::String(unit));
        m.insert(
            "estimate".into(),
            estimate.map_or(Value::Null, estimate_json),
        );
        m.insert("real".into(), actual.map_or(Value::Null, |a| a.json));
        m.insert("pct".into(), opt_num(budget.and_then(|b| b.pct)));
        m.insert(
            "level".into(),
            budget.map_or(Value::Null, |b| Value::String(b.level.as_str().into())),
        );
        m
    }

    /// `plan()` (`src/panel/moirai.ts:104`): estimado × real por plano e unidade.
    pub fn plan(&mut self) -> Value {
        let (warn_pct, ask_pct) = self.limits();
        let mut plans = Vec::new();
        for registry in (self.deps.registries)() {
            let record = read_json(&registry.orch.join("plan.json"));
            let path = record
                .as_ref()
                .map_or_else(String::new, |r| text(r.get("path")));
            let body = if path.is_empty() {
                None
            } else {
                read_text(Path::new(&path))
            };
            let (Some(record), Some(body)) = (record, body) else {
                continue;
            };
            let agents = agents_of(&registry);
            let rows: Vec<Value> = parse_section(&body)
                .iter()
                .map(|estimate| {
                    let agent = agents.iter().find(|a| a.name == estimate.unit);
                    Value::Object(self.row(&registry, Some(estimate), agent))
                })
                .collect();
            let mut m = Map::new();
            m.insert("registry".into(), Value::String(registry.slug.clone()));
            m.insert("path".into(), Value::String(path));
            m.insert(
                "approved".into(),
                Value::Bool(plan_sha(&body) == text(record.get("sha"))),
            );
            m.insert("units".into(), Value::Array(rows));
            plans.push(Value::Object(m));
        }
        let mut out = Map::new();
        out.insert("v".into(), json_num(1.0));
        out.insert("warnPct".into(), json_num(warn_pct));
        out.insert("askPct".into(), json_num(ask_pct));
        out.insert("plans".into(), Value::Array(plans));
        Value::Object(out)
    }

    /// `unit(name, registry)` (`src/panel/moirai.ts:121`): 400 para nome inválido, 404 para nome fora dos registros.
    pub fn unit(&mut self, name: &str, registry: Option<&str>) -> Result<Value, MoiraiError> {
        if !unit_name_ok(name) {
            return Err(MoiraiError {
                status: 400,
                code: ERR_INVALID_UNIT,
                message: "nome de unidade inválido".into(),
            });
        }
        let registries: Vec<Registry> = (self.deps.registries)()
            .into_iter()
            .filter(|r| registry.is_none_or(|slug| r.slug == slug))
            .collect();
        for r in registries {
            let agents = agents_of(&r);
            let Some(agent) = agents.iter().find(|a| a.name == name) else {
                continue;
            };
            let record = read_json(&r.orch.join("plan.json"));
            let body = record
                .as_ref()
                .map(|rec| text(rec.get("path")))
                .filter(|p| !p.is_empty())
                .and_then(|p| read_text(Path::new(&p)));
            let estimate = body.as_deref().and_then(|b| estimate_of_unit(b, name));
            let (warn_pct, ask_pct) = self.limits();
            let mut out = Map::new();
            out.insert("v".into(), json_num(1.0));
            out.insert("warnPct".into(), json_num(warn_pct));
            out.insert("askPct".into(), json_num(ask_pct));
            out.insert("registry".into(), Value::String(r.slug.clone()));
            out.extend(self.row(&r, estimate.as_ref(), Some(agent)));
            return Ok(Value::Object(out));
        }
        Err(MoiraiError {
            status: 404,
            code: ERR_UNKNOWN_UNIT,
            message: "unidade desconhecida".into(),
        })
    }
}

#[cfg(test)]
mod tests;
