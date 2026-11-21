//! Consumo de tokens a partir dos transcripts locais do Claude Code (`src/lib/usage.ts:84-243, 643-668`): uma linha por
//! (sessão, subagente?, dia), com custo, polling e reescrita de contexto.

pub mod guard_rules;
pub mod turn_class;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use hades_core::fsx::read_text;
use indexmap::IndexMap;
use serde_json::{Map, Value};

pub use guard_rules::GuardRules;
pub use turn_class::{TurnKind, TurnTool, bash_is_follow_up, classify_turn, is_wake_text};

use crate::price::{Tok, model_key, price_of};

pub const CACHE_VERSION: u32 = 6;
const REWRITE_GAP_SECONDS: f64 = 3600.0;
const REWRITE_MIN_TOKENS: f64 = 100_000.0;

pub type Dict = Map<String, Value>;

/// Só na linha que contém a primeira resposta da sessão principal: o contexto fixo.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub ctx: f64,
    pub model: String,
}

/// Uma linha por (sessão, subagente?, dia) (`Row`, `usage.ts:32`).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub session: String,
    pub sidechain: bool,
    pub day: String,
    pub cwd: String,
    pub cost: f64,
    pub turns: u64,
    pub poll_turns: u64,
    pub poll_cost: f64,
    pub rewrite_cost: f64,
    pub ctx_sum: f64,
    pub ctx_max: f64,
    pub follow_turns: Option<u64>,
    pub follow_cost: Option<f64>,
    pub follow_ctx_sum: Option<f64>,
    pub models: IndexMap<String, u64>,
    pub efforts: IndexMap<String, u64>,
    pub read: f64,
    pub write: f64,
    pub output: f64,
    pub first: String,
    pub last: String,
    pub tok: IndexMap<String, Tok>,
    pub open: Option<Open>,
}

/// Uma requisição com horário: base da calibração dos limites (`Req`, `usage.ts:73`).
#[derive(Clone, Debug, PartialEq)]
pub struct Req {
    pub epoch_s: i64,
    pub model: String,
    pub tok: Tok,
    pub cost: f64,
}

struct Request {
    usage: Option<Dict>,
    model: Option<&'static str>,
    ts: String,
    sid: String,
    side: bool,
    cwd: String,
    effort: String,
    poll: bool,
    tools: Vec<TurnTool>,
    after_wake: bool,
}

/// `value || fallback` de Python/JS: vazio é ausente, zero, `false`, texto e coleção vazios.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// `Boolean(value)` do JS: coleção vazia é verdadeira.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn num(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

/// `text(value, fallback)` (`usage.ts:26`): o valor quando é texto não vazio, senão o padrão.
fn text(value: Option<&Value>, fallback: &str) -> String {
    match value {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => fallback.to_owned(),
    }
}

fn dict(value: Option<&Value>) -> Option<&Dict> {
    value.and_then(Value::as_object)
}

/// (custo da requisição, parte de escrita de cache) em US$ (`cost`, `usage.ts:83`). `None` para chave fora da tabela.
pub fn cost(model_key_name: &str, usage: &Dict) -> Option<(f64, f64)> {
    let [in_price, out_price, read_price] = price_of(model_key_name)?;
    let creation = dict(usage.get("cache_creation"));
    let no_breakdown = creation.is_none_or(Map::is_empty);
    let write_5m = if no_breakdown {
        num(usage.get("cache_creation_input_tokens"))
    } else {
        num(creation.and_then(|c| c.get("ephemeral_5m_input_tokens")))
    };
    let write_1h = num(creation.and_then(|c| c.get("ephemeral_1h_input_tokens")));
    let write = (write_5m * in_price * 1.25 + write_1h * in_price * 2.0) / 1e6;
    let total = (num(usage.get("input_tokens")) * in_price
        + num(usage.get("output_tokens")) * out_price
        + num(usage.get("cache_read_input_tokens")) * read_price)
        / 1e6
        + write;
    Some((total, write))
}

/// Os `tool_use` do bloco da mensagem, na ordem; o Bash leva o comando (`toolsOf`, `usage.ts:120`).
fn tools_of(message: &Dict) -> Vec<TurnTool> {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| {
            let block = block.as_object()?;
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            let name = block.get("name")?.as_str()?;
            let command = dict(block.get("input"))
                .and_then(|input| input.get("command"))
                .and_then(Value::as_str);
            Some(TurnTool {
                name: name.to_owned(),
                command: command.map(str::to_owned),
            })
        })
        .collect()
}

/// O texto do usuário é o aviso de segundo plano: a primeira parte de texto da mensagem (`userWakes`, `usage.ts:132`).
fn user_wakes(message: &Dict) -> bool {
    match message.get("content") {
        Some(Value::String(content)) => is_wake_text(content),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(Value::as_object)
            .find_map(|block| block.get("text").and_then(Value::as_str))
            .is_some_and(is_wake_text),
        _ => false,
    }
}

/// `foregroundPolling` (`usage.ts:140`).
fn foreground_polling(message: &Dict, rules: &GuardRules) -> bool {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return false;
    };
    blocks.iter().any(|block| {
        let Some(block) = block.as_object() else {
            return false;
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_use")
            || block.get("name").and_then(Value::as_str) != Some("Bash")
        {
            return false;
        }
        let Some(input) = dict(block.get("input")) else {
            return false;
        };
        match input.get("command").and_then(Value::as_str) {
            Some(command) if !command.is_empty() => {
                rules.is_polling(command) && !truthy(input.get("run_in_background"))
            }
            _ => false,
        }
    })
}

/// Chave de agrupamento do `Map` do JS para o id do pedido (igualdade de valor).
fn id_key(value: &Value) -> String {
    match value {
        Value::Null => "n".to_owned(),
        Value::String(s) => format!("s:{s}"),
        Value::Number(n) => format!("d:{}", n.as_f64().unwrap_or(f64::NAN)),
        other => format!("j:{other}"),
    }
}

fn new_request(line: &Dict, message: &Dict, path: &str, after_wake: bool) -> Request {
    Request {
        usage: dict(message.get("usage")).cloned(),
        model: model_key(message.get("model").and_then(Value::as_str)),
        ts: text(line.get("timestamp"), ""),
        sid: text(line.get("sessionId"), path),
        side: truthy(line.get("isSidechain")) || path.contains("/subagents/"),
        cwd: text(line.get("cwd"), ""),
        effort: text(line.get("effort"), "?"),
        poll: false,
        tools: Vec::new(),
        after_wake,
    }
}

/// `requestsOf` (`usage.ts:149`): um `Request` por `requestId` (ou `message.id`), na ordem da primeira aparição.
fn requests_of(path: &Path, rules: &GuardRules) -> Vec<Request> {
    let Some(content) = read_text(path) else {
        return Vec::new();
    };
    let path_text = path.to_string_lossy();
    let mut requests: IndexMap<String, Request> = IndexMap::new();
    let mut wake_pending = false;
    for raw in content.split('\n') {
        let may_wake = raw.contains("<task-notification") && raw.contains("\"user\"");
        if !may_wake && !raw.contains("\"assistant\"") {
            continue;
        }
        let Ok(Value::Object(line)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let line_type = line.get("type").and_then(Value::as_str);
        let empty = Map::new();
        let message = dict(line.get("message")).unwrap_or(&empty);
        if line_type == Some("user") {
            wake_pending = user_wakes(message);
            continue;
        }
        if line_type != Some("assistant") {
            continue;
        }
        let null = Value::Null;
        let request_id = line.get("requestId").unwrap_or(&null);
        let id = if is_empty(request_id) {
            message.get("id").unwrap_or(&null)
        } else {
            request_id
        };
        let key = id_key(if is_empty(id) { &null } else { id });
        let request = requests
            .entry(key)
            .or_insert_with(|| new_request(&line, message, &path_text, wake_pending));
        wake_pending = false;
        request.tools.extend(tools_of(message));
        if foreground_polling(message, rules) {
            request.poll = true;
        }
    }
    requests.into_values().collect()
}

fn new_row(request: &Request, day: &str) -> Row {
    Row {
        session: request.sid.clone(),
        sidechain: request.side,
        day: day.to_owned(),
        cwd: request.cwd.clone(),
        cost: 0.0,
        turns: 0,
        poll_turns: 0,
        poll_cost: 0.0,
        rewrite_cost: 0.0,
        ctx_sum: 0.0,
        ctx_max: 0.0,
        follow_turns: Some(0),
        follow_cost: Some(0.0),
        follow_ctx_sum: Some(0.0),
        models: IndexMap::new(),
        efforts: IndexMap::new(),
        read: 0.0,
        write: 0.0,
        output: 0.0,
        first: request.ts.clone(),
        last: request.ts.clone(),
        tok: IndexMap::new(),
        open: None,
    }
}

pub fn add_tok(a: Option<Tok>, b: Tok) -> Tok {
    match a {
        Some(a) => [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]],
        None => b,
    }
}

fn civil_days(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn digits(s: &str, len: usize) -> Option<i64> {
    if s.len() == len && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// `Date.parse` dos horários ISO do transcript, em ms; `NaN` quando não é data. Sem fuso vale UTC.
pub fn parse_ts_ms(ts: &str) -> f64 {
    parse_ts(ts).map_or(f64::NAN, |ms| ms as f64)
}

fn parse_ts(ts: &str) -> Option<i64> {
    let (date, time) = match ts.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (ts, None),
    };
    let mut parts = date.split('-');
    let (y, m, d) = (
        digits(parts.next()?, 4)?,
        digits(parts.next()?, 2)?,
        digits(parts.next()?, 2)?,
    );
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut ms = civil_days(y, m, d) * 86_400_000;
    let Some(time) = time else {
        return Some(ms);
    };
    let (clock, offset_min) = if let Some(c) = time.strip_suffix('Z') {
        (c, 0)
    } else if let Some(i) = time.rfind(['+', '-']) {
        let (c, off) = time.split_at(i);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':')?;
        (c, sign * (digits(oh, 2)? * 60 + digits(om, 2)?))
    } else {
        (time, 0)
    };
    let (hms, frac) = match clock.split_once('.') {
        Some((h, f)) => (h, Some(f)),
        None => (clock, None),
    };
    let mut t = hms.split(':');
    let hh = digits(t.next()?, 2)?;
    let mm = digits(t.next()?, 2)?;
    let ss = t.next().map_or(Some(0), |s| digits(s, 2))?;
    if t.next().is_some() || hh > 24 || mm > 59 || ss > 59 {
        return None;
    }
    ms += (hh * 3600 + mm * 60 + ss) * 1000;
    if let Some(f) = frac {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let padded = format!("{:0<3}", &f[..f.len().min(3)]);
        ms += padded.parse::<i64>().ok()?;
    }
    Some(ms - offset_min * 60_000)
}

fn bump(counter: &mut IndexMap<String, u64>, key: &str) {
    *counter.entry(key.to_owned()).or_insert(0) += 1;
}

/// Agrega um transcript por (sessão, subagente?, dia), na ordem do horário das requisições (`parseFile`,
/// `usage.ts:200`).
pub fn parse_file(path: &Path, rules: &GuardRules) -> Vec<Row> {
    parse_file_full(path, rules, 0.0).0
}

/// As linhas de `parse_file` e as requisições com horário, só as de `since_ms` em diante (`parseFileFull`,
/// `usage.ts:205`).
pub fn parse_file_full(path: &Path, rules: &GuardRules, since_ms: f64) -> (Vec<Row>, Vec<Req>) {
    let mut reqs = Vec::new();
    let mut rows: IndexMap<(String, bool, String), Row> = IndexMap::new();
    let mut previous: IndexMap<String, f64> = IndexMap::new();
    let mut opened: HashSet<String> = HashSet::new();
    let mut ordered = requests_of(path, rules);
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts));
    for request in ordered {
        let Some(usage) = request.usage.as_ref().filter(|u| !u.is_empty()) else {
            continue;
        };
        let Some(model) = request.model else {
            continue;
        };
        if request.ts.is_empty() {
            continue;
        }
        let Some((price, write)) = cost(model, usage) else {
            continue;
        };
        let input = num(usage.get("input_tokens"));
        let output = num(usage.get("output_tokens"));
        let read = num(usage.get("cache_read_input_tokens"));
        let creation = num(usage.get("cache_creation_input_tokens"));
        let ctx = input + read + creation;
        let day: String = request.ts.chars().take(10).collect();
        let key = (request.sid.clone(), request.side, day.clone());
        let row = rows.entry(key).or_insert_with(|| new_row(&request, &day));
        if !request.side && opened.insert(request.sid.clone()) {
            row.open = Some(Open {
                ctx,
                model: model.to_owned(),
            });
        }
        row.cost += price;
        row.turns += 1;
        row.ctx_sum += ctx;
        row.ctx_max = row.ctx_max.max(ctx);
        row.last = request.ts.clone();
        row.read += read;
        row.write += creation;
        row.output += output;
        let tok: Tok = [input, output, creation, read];
        let sum = add_tok(row.tok.get(model).copied(), tok);
        row.tok.insert(model.to_owned(), sum);
        let when = parse_ts_ms(&request.ts);
        if when >= since_ms {
            reqs.push(Req {
                epoch_s: (when / 1000.0).floor() as i64,
                model: model.to_owned(),
                tok,
                cost: price,
            });
        }
        bump(&mut row.models, model);
        bump(&mut row.efforts, &request.effort);
        if classify_turn(&request.tools, request.after_wake) == TurnKind::Follow {
            row.follow_turns = Some(row.follow_turns.unwrap_or(0) + 1);
            row.follow_cost = Some(row.follow_cost.unwrap_or(0.0) + price);
            row.follow_ctx_sum = Some(row.follow_ctx_sum.unwrap_or(0.0) + ctx);
        }
        if request.poll {
            row.poll_turns += 1;
            row.poll_cost += price;
        }
        if let Some(before) = previous.get(&request.sid)
            && (when - before) / 1000.0 > REWRITE_GAP_SECONDS
            && creation > REWRITE_MIN_TOKENS
        {
            row.rewrite_cost += write;
        }
        previous.insert(request.sid.clone(), when);
    }
    (rows.into_values().collect(), reqs)
}

/// Transcripts (`.jsonl` em qualquer nível) de uma pasta, sem seguir links simbólicos, em ordem estável
/// (`transcriptsIn`, `usage.ts:643`).
pub fn transcripts_in(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                walk(&path, found);
            } else if kind.is_file() && entry.file_name().to_string_lossy().ends_with(".jsonl") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    found
}

#[cfg(test)]
mod tests;
