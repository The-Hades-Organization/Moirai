//! Histórico das unidades encerradas: registros dos agentes, transcripts, histórico do gate e git
//! (`src/commands/moirai.ts:57-98, 159-213`, `src/lib/moirai.ts:114-223, 438-458`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hades_core::Env;
use hades_core::clock::now_ms;
use hades_core::config::config_value;
use hades_core::fsx::{is_dir, list_dir, read_text};
use hades_core::jsfmt::to_fixed;
use hades_core::jsonfmt::{json_compact, json_indent};
use hades_core::output::{HadesError, warn};
use hades_core::paths::{skill_root, work_root};
use hades_core::proc::{RunOptions, run};
use hades_core::registry::iso_from_epoch;
use serde_json::{Map, Number, Value, json};

use crate::jsnum::js_number;
use crate::model::{HistoryUnit, Mode, Params};
use crate::real::{default_projects_dir, encode_cwd};
use crate::stats::band_of;
use crate::transcripts::guard_rules::GuardRules;
use crate::transcripts::{parse_file, parse_ts_ms, transcripts_in};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_MAX_BUFFER: usize = 64 << 20;
const ROOT_MAQUINA: &str = "_maquina";
const DASH: &str = "—";
const DAY_MS: f64 = 86_400_000.0;

/// `rootsOf` (`src/commands/moirai.ts:59`): raízes do work root, menos `_maquina`, que têm `orchestration/agents/`.
fn roots_of(base: &Path) -> Vec<String> {
    list_dir(base)
        .into_iter()
        .filter(|n| n != ROOT_MAQUINA && is_dir(&base.join(n).join("orchestration").join("agents")))
        .collect()
}

/// `recordsOf` (`src/commands/moirai.ts:67`): cada `agents/*.json` em ordem de byte; o ilegível avisa e segue.
fn records_of(base: &Path, root: &str) -> Vec<Map<String, Value>> {
    let dir = base.join(root).join("orchestration").join("agents");
    let mut records = Vec::new();
    for file in list_dir(&dir).into_iter().filter(|n| n.ends_with(".json")) {
        match serde_json::from_str::<Value>(&read_text(&dir.join(&file)).unwrap_or_default()) {
            Ok(Value::Object(doc)) => records.push(doc),
            Ok(_) => {}
            Err(_) => warn(&format!(
                "moirai: registro ilegível ignorado: {root}/{file}"
            )),
        }
    }
    records
}

fn str_of(record: &Map<String, Value>, key: &str) -> String {
    match record.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// `str(a) || str(b)` do TS.
fn first_str(record: &Map<String, Value>, keys: [&str; 2]) -> String {
    let first = str_of(record, keys[0]);
    if first.is_empty() {
        str_of(record, keys[1])
    } else {
        first
    }
}

/// `count` (`src/lib/moirai.ts:122`): número finito maior ou igual a zero.
fn count_of(record: &Map<String, Value>, key: &str) -> Option<f64> {
    record
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
}

/// `String(x)` do JS para um valor de JSON.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(_) => json_compact(value),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `String(record[a] ?? record[b] ?? "")` (`src/commands/moirai.ts:195`).
fn nullish_string(record: &Map<String, Value>, keys: [&str; 2]) -> String {
    for key in keys {
        match record.get(key) {
            None | Some(Value::Null) => {}
            Some(v) => return js_string(v),
        }
    }
    String::new()
}

/// `isEnded` (`src/lib/moirai.ts:125`).
fn is_ended(record: &Map<String, Value>) -> bool {
    matches!(
        record.get("state").and_then(Value::as_str),
        Some("done" | "archived")
    )
}

/// `isRealUnit` (`src/lib/moirai.ts:146`): encerrada e com rastro de trabalho de verdade (transcript com custo ou
/// worktree que ainda existe).
pub fn is_real_unit(
    record: &Map<String, Value>,
    has_transcript: bool,
    worktree_exists: bool,
) -> bool {
    is_ended(record) && !str_of(record, "name").is_empty() && (has_transcript || worktree_exists)
}

/// `seconds` (`src/lib/moirai.ts:148`).
pub fn seconds(from: &str, to: &str) -> Option<f64> {
    let (a, b) = (parse_ts_ms(from), parse_ts_ms(to));
    if a.is_nan() || b.is_nan() || b < a {
        None
    } else {
        Some((b - a) / 1000.0)
    }
}

/// `modeOf` (`src/lib/moirai.ts:154`).
pub fn mode_of(value: Option<&Value>) -> Mode {
    if value.and_then(Value::as_str) == Some("skill") {
        Mode::Skill
    } else {
        Mode::Projeto
    }
}

/// O medido de uma unidade (`Measured`, `src/lib/moirai.ts:107`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    pub cost_usd: Option<f64>,
    pub tokens: Option<f64>,
    pub gate_s: f64,
    pub gate_peak_mb: Option<f64>,
    pub files: Option<f64>,
    pub lines: Option<f64>,
}

/// `historyUnit` (`src/lib/moirai.ts:160`).
pub fn history_unit(
    record: &Map<String, Value>,
    root: &str,
    measured: &Measured,
    params: &Params,
) -> HistoryUnit {
    let created_at = first_str(record, ["createdAt", "adoptedAt"]);
    let done_at = first_str(record, ["doneAt", "archivedAt"]);
    let role = str_of(record, "role");
    HistoryUnit {
        name: str_of(record, "name"),
        root: root.to_owned(),
        mode: mode_of(record.get("mode")),
        role: if role.is_empty() {
            "impl".to_owned()
        } else {
            role
        },
        state: str_of(record, "state"),
        wall_s: seconds(&created_at, &done_at),
        created_at,
        done_at,
        cost_usd: measured.cost_usd,
        tokens: measured.tokens,
        gate_s: measured.gate_s,
        gate_peak_mb: measured.gate_peak_mb,
        clears: count_of(record, "clears").unwrap_or(0.0),
        resumes: count_of(record, "resumes"),
        files: measured.files,
        lines: measured.lines,
        band: match measured.lines {
            Some(lines) if lines != 0.0 => Some(band_of(lines, params)),
            _ => None,
        },
    }
}

/// `dedupe` (`src/lib/moirai.ts:185`): a mesma unidade (nome e `createdAt`) em duas raízes fica com a de `doneAt` mais
/// novo, na posição da primeira; saída por `doneAt` e nome.
pub fn dedupe(units: Vec<HistoryUnit>) -> Vec<HistoryUnit> {
    let mut best: Vec<HistoryUnit> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for unit in units {
        let key = (unit.name.clone(), unit.created_at.clone());
        match index.get(&key) {
            Some(&i) => {
                if unit.done_at > best[i].done_at {
                    best[i] = unit;
                }
            }
            None => {
                index.insert(key, best.len());
                best.push(unit);
            }
        }
    }
    best.sort_by(|a, b| a.done_at.cmp(&b.done_at).then_with(|| a.name.cmp(&b.name)));
    best
}

/// `inWindow` (`src/lib/moirai.ts:196`): só o encerrado nos últimos `days` dias antes de `now_ms` (0 = tudo).
pub fn in_window(units: Vec<HistoryUnit>, now_ms: i64, days: f64) -> Vec<HistoryUnit> {
    if days <= 0.0 {
        return units;
    }
    let since_ms = now_ms - (days * DAY_MS) as i64;
    let iso = iso_from_epoch(since_ms.div_euclid(1000));
    let since = format!(
        "{}.{:03}Z",
        iso.trim_end_matches('Z'),
        since_ms.rem_euclid(1000)
    );
    units.into_iter().filter(|u| u.done_at >= since).collect()
}

fn digits_only(text: &str) -> Option<f64> {
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

/// `gateOfLabel` (`src/lib/moirai.ts:203`): tempo e pico de gate de um label no histórico do gate-lock (linhas `done`).
pub fn gate_of_label(journals: &[String], label: &str) -> (f64, Option<f64>) {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut gate_s = 0.0;
    let mut peak: Option<f64> = None;
    for journal in journals {
        for line in journal.split('\n') {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.get(1) != Some(&"done") || fields.get(3) != Some(&label) || !seen.insert(line)
            {
                continue;
            }
            for field in fields.iter().skip(2) {
                if let Some(n) = field
                    .strip_prefix("dur=")
                    .and_then(|r| r.strip_suffix('s'))
                    .and_then(digits_only)
                {
                    gate_s += n;
                }
                if let Some(n) = field
                    .strip_prefix("peak=")
                    .and_then(|r| r.strip_suffix("MB"))
                    .and_then(digits_only)
                {
                    peak = Some(peak.unwrap_or(0.0).max(n));
                }
            }
        }
    }
    (gate_s, peak)
}

/// `usageOf` (`src/commands/moirai.ts:82`): custo e tokens (leitura de cache + escrita de cache + saída) das sessões
/// do worktree.
fn usage_of(env: &Env, rules: &GuardRules, worktree: &str) -> Option<(f64, f64)> {
    if worktree.is_empty() {
        return None;
    }
    let dir = default_projects_dir(env).join(encode_cwd(worktree));
    let (mut cost, mut tokens, mut any) = (0.0, 0.0, false);
    for path in transcripts_in(&dir) {
        for row in parse_file(&path, rules) {
            if row.turns == 0 {
                continue;
            }
            any = true;
            cost += row.cost;
            tokens += row.read + row.write + row.output;
        }
    }
    any.then_some((cost, tokens))
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let repo = repo.to_string_lossy();
    let mut full = vec!["-C", repo.as_ref()];
    full.extend_from_slice(args);
    let result = run(
        "git",
        &full,
        &RunOptions {
            timeout: Some(GIT_TIMEOUT),
            max_buffer: Some(GIT_MAX_BUFFER),
            ..Default::default()
        },
    );
    (result.status == 0).then_some(result.stdout)
}

/// `sizeOf` (`src/commands/moirai.ts:169`): arquivos distintos e linhas (adicionadas + removidas) dos commits da própria
/// branch (primeiro pai, sem merges) entre o início e o fim da unidade; `None` se a branch não existe mais.
fn size_of(repo: &Path, branch: &str, since: &str, until: &str) -> Option<(f64, f64)> {
    if branch.is_empty() {
        return None;
    }
    let reference = format!("refs/heads/{branch}");
    git(repo, &["rev-parse", "--verify", "--quiet", &reference])?;
    let since_arg = format!("--since={since}");
    let until_arg = format!("--until={until}");
    let mut args = vec![
        "log",
        "--first-parent",
        "--no-merges",
        "--numstat",
        "--format=",
    ];
    if !since.is_empty() {
        args.push(&since_arg);
    }
    if !until.is_empty() {
        args.push(&until_arg);
    }
    args.push(&reference);
    let log = git(repo, &args)?;
    let mut files: HashSet<String> = HashSet::new();
    let mut lines = 0.0;
    for line in log.split('\n') {
        let mut parts = line.split('\t');
        let (Some(added), Some(removed)) = (parts.next(), parts.next()) else {
            continue;
        };
        let path: Vec<&str> = parts.collect();
        if path.is_empty() {
            continue;
        }
        files.insert(path.join("\t"));
        lines += digits_only(added).unwrap_or(0.0) + digits_only(removed).unwrap_or(0.0);
    }
    Some((files.len() as f64, lines))
}

/// `gateJournals` (`src/commands/moirai.ts:185`).
fn gate_journals(base: &Path, roots: &[String]) -> Vec<String> {
    std::iter::once(base.join(ROOT_MAQUINA))
        .chain(roots.iter().map(|r| base.join(r)))
        .map(|d| read_text(&d.join("locks").join("gate-journal.tsv")).unwrap_or_default())
        .filter(|t| !t.is_empty())
        .collect()
}

/// `intKey` da janela (`HADES_MOIRAI_WINDOW_DAYS`, `src/commands/moirai.ts:190`).
pub fn window_days(env: &Env, cwd: &Path) -> Result<f64, HadesError> {
    let key = "HADES_MOIRAI_WINDOW_DAYS";
    let raw = config_value(env, cwd, key, "90");
    match js_number(&raw) {
        Some(v) if v.is_finite() && v.fract() == 0.0 && v >= 0.0 => Ok(v),
        _ => Err(HadesError::new(format!(
            "{key}={raw}: esperava um inteiro maior ou igual a zero"
        ))),
    }
}

/// `collectHistory` (`src/commands/moirai.ts:189`): todas as unidades encerradas de raiz real, com o medido; só leitura.
pub fn collect_history(
    env: &Env,
    cwd: &Path,
    params: &Params,
) -> Result<Vec<HistoryUnit>, HadesError> {
    let window = window_days(env, cwd)?;
    let rules = GuardRules::load(env).map_err(HadesError::new)?;
    let base = work_root(env);
    let roots = roots_of(&base);
    let journals = gate_journals(&base, &roots);
    let skill = skill_root(env);
    let mut units = Vec::new();
    for root in &roots {
        for record in records_of(&base, root) {
            let name = str_of(&record, "name");
            let worktree = str_of(&record, "worktree");
            let branch = str_of(&record, "branch");
            let usage = usage_of(env, &rules, &worktree);
            let worktree_exists = !worktree.is_empty() && is_dir(Path::new(&worktree));
            if !is_real_unit(&record, usage.is_some(), worktree_exists) {
                continue;
            }
            let (gate_s, gate_peak_mb) = gate_of_label(&journals, &name);
            let repo: PathBuf = if mode_of(record.get("mode")) == Mode::Skill {
                skill.clone()
            } else {
                PathBuf::from(&worktree)
            };
            let created = nullish_string(&record, ["createdAt", "adoptedAt"]);
            let done = nullish_string(&record, ["doneAt", "archivedAt"]);
            let size = if !repo.as_os_str().is_empty() && is_dir(&repo) {
                size_of(&repo, &branch, &created, &done)
            } else {
                None
            };
            let measured = Measured {
                cost_usd: usage.map(|u| u.0),
                tokens: usage.map(|u| u.1),
                gate_s,
                gate_peak_mb,
                files: size.map(|s| s.0),
                lines: size.map(|s| s.1),
            };
            units.push(history_unit(&record, root, &measured, params));
        }
    }
    Ok(in_window(dedupe(units), now_ms(env), window))
}

fn number(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn optional(value: Option<f64>) -> Value {
    value.map_or(Value::Null, number)
}

/// A unidade na ordem de chaves do objeto do TS.
pub fn unit_value(u: &HistoryUnit) -> Value {
    json!({
        "name": u.name,
        "root": u.root,
        "mode": u.mode.as_str(),
        "role": u.role,
        "state": u.state,
        "createdAt": u.created_at,
        "doneAt": u.done_at,
        "costUsd": optional(u.cost_usd),
        "tokens": optional(u.tokens),
        "wallS": optional(u.wall_s),
        "gateS": number(u.gate_s),
        "gatePeakMb": optional(u.gate_peak_mb),
        "clears": number(u.clears),
        "resumes": optional(u.resumes),
        "files": optional(u.files),
        "lines": optional(u.lines),
        "band": u.band.map_or(Value::Null, |b| Value::String(b.as_str().to_owned())),
    })
}

/// `JSON.stringify({ v: 1, units }, null, 1)` (`src/commands/moirai.ts:268`).
pub fn history_json(units: &[HistoryUnit]) -> String {
    json_indent(
        &json!({ "v": 1, "units": units.iter().map(unit_value).collect::<Vec<_>>() }),
        1,
    )
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn pad(value: &str, width: usize, right: bool) -> String {
    let fill = " ".repeat(width.saturating_sub(utf16_len(value)));
    if right {
        format!("{fill}{value}")
    } else {
        format!("{value}{fill}")
    }
}

/// `slice(0, n)` por unidade UTF-16.
fn slice_utf16(text: &str, n: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        used += c.len_utf16();
        if used > n {
            break;
        }
        out.push(c);
    }
    out
}

fn num(value: Option<f64>, f: impl Fn(f64) -> String) -> String {
    value.map_or_else(|| DASH.to_owned(), f)
}

/// `historyLines` (`src/lib/moirai.ts:440`).
pub fn history_lines(units: &[HistoryUnit]) -> Vec<String> {
    let head = format!(
        "{}{}{}{}{}{}{}{}{}{}{}  tam",
        pad("unidade", 26, false),
        pad("papel", 12, false),
        pad("modo", 8, false),
        pad("custo", 8, true),
        pad("tokens", 9, true),
        pad("relógio", 9, true),
        pad("gate", 7, true),
        pad("pico", 8, true),
        pad("clr", 4, true),
        pad("arq", 5, true),
        pad("linhas", 8, true),
    );
    let mut lines = vec![head];
    for u in units {
        lines.push(format!(
            "{}{}{}{}{}{}{}{}{}{}{}  {}",
            pad(&slice_utf16(&u.name, 25), 26, false),
            pad(&u.role, 12, false),
            pad(u.mode.as_str(), 8, false),
            pad(
                &num(u.cost_usd, |n| format!("${}", to_fixed(n, 2))),
                8,
                true
            ),
            pad(
                &num(u.tokens, |n| format!("{}M", to_fixed(n / 1e6, 1))),
                9,
                true
            ),
            pad(
                &num(u.wall_s, |n| format!("{}m", to_fixed(n / 60.0, 0))),
                9,
                true
            ),
            pad(&format!("{}m", to_fixed(u.gate_s / 60.0, 0)), 7, true),
            pad(
                &num(u.gate_peak_mb, |n| format!("{}G", to_fixed(n / 1024.0, 1))),
                8,
                true
            ),
            pad(&json_compact(&number(u.clears)), 4, true),
            pad(&num(u.files, |n| json_compact(&number(n))), 5, true),
            pad(&num(u.lines, |n| json_compact(&number(n))), 8, true),
            u.band.map_or(DASH, |b| b.as_str()),
        ));
    }
    lines
}

#[cfg(test)]
mod tests;
