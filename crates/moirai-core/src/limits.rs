//! Limites de uso do plano (janela de 5 h e semanal): amostras do % medido, calibração tokens → % e conversão
//! (`src/lib/limites.ts`, borda de leitura de `src/lib/limites-io.ts:15-96` e `limitsNow` de `src/commands/usage.ts:332`).
//!
//! Só leitura: o Rust não grava `samples.jsonl` nem `usage/cache.json` (decisão do dono, plano §4.3-1). Sem amostra
//! não há leitura de transcript.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use hades_core::config::config_value;
use hades_core::env::Env;
use hades_core::fsx::read_text;
use hades_core::jsfmt::to_fixed;
use hades_core::output::{HadesError, warn};
use hades_core::paths::hades_home;
use hades_core::registry::iso_from_epoch;
use indexmap::IndexMap;
use serde_json::{Map, Number, Value};

use crate::jsnum::js_number;
use crate::price::Tok;
use crate::transcripts::{GuardRules, Req, parse_file_full, parse_ts_ms, transcripts_in};

const SEGUNDOS_DE_FOLGA_DA_RENOVACAO: f64 = 120.0;
const PASSADAS_DO_AJUSTE: usize = 400;
const TOKENS_POR_MILHAO: f64 = 1e6;
const MS_POR_DIA: f64 = 86_400_000.0;
const PORCENTO: f64 = 100.0;

/// `Janela` (`limites.ts:6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Janela {
    Five,
    Seven,
}

/// `JANELAS` (`limites.ts:7`), na ordem de declaração.
pub const JANELAS: [Janela; 2] = [Janela::Five, Janela::Seven];

impl Janela {
    /// A chave do JSON (`five`, `seven`).
    pub fn key(self) -> &'static str {
        match self {
            Janela::Five => "five",
            Janela::Seven => "seven",
        }
    }

    /// `ROTULO` (`limites.ts:8`).
    pub fn rotulo(self) -> &'static str {
        match self {
            Janela::Five => "5 h",
            Janela::Seven => "semana",
        }
    }
}

/// `Leitura` (`limites.ts:11`): % usado (0–100) de uma janela e quando ela renova (epoch s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leitura {
    pub pct: f64,
    pub resets_at: f64,
}

/// `Amostra` (`limites.ts:17`).
#[derive(Clone, Debug, PartialEq)]
pub struct Amostra {
    /// Instante da leitura, UTC (ISO 8601 com Z).
    pub at: String,
    pub five: Option<Leitura>,
    pub seven: Option<Leitura>,
    pub version: Option<String>,
}

impl Amostra {
    pub fn janela(&self, janela: Janela) -> Option<Leitura> {
        match janela {
            Janela::Five => self.five,
            Janela::Seven => self.seven,
        }
    }
}

/// `Opcoes` (`limites.ts:25`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opcoes {
    /// Menor variação de % que vale como intervalo (a leitura pode ser inteira).
    pub min_delta_pct: f64,
    /// Intervalos mínimos para calibrar (por modelo e no total).
    pub min_intervals: f64,
    /// Peso de um token de leitura de cache frente a um token novo (0–1).
    pub cache_read_weight: f64,
}

fn leitura(value: Option<&Value>) -> Option<Leitura> {
    let map = value?.as_object()?;
    let pct = map.get("used_percentage")?.as_f64()?;
    let resets_at = map.get("resets_at")?.as_f64()?;
    if !pct.is_finite() || !resets_at.is_finite() || pct < 0.0 {
        return None;
    }
    Some(Leitura { pct, resets_at })
}

/// `new Date(ms).toISOString()` (ms inteiros).
fn iso_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let frac = ms.rem_euclid(1000);
    let base = iso_from_epoch(secs);
    format!("{}.{frac:03}Z", base.trim_end_matches('Z'))
}

/// `amostraDoStatusLine` (`limites.ts:48`): amostra do JSON que o Claude Code entrega ao status line; sem
/// `rate_limits` não há amostra.
pub fn amostra_do_status_line(text: &str, now_epoch_ms: i64) -> Option<Amostra> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let limits = doc.as_object()?.get("rate_limits")?.as_object()?;
    let five = leitura(limits.get("five_hour"));
    let seven = leitura(limits.get("seven_day"));
    if five.is_none() && seven.is_none() {
        return None;
    }
    let version = doc
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    Some(Amostra {
        at: iso_ms(now_epoch_ms),
        five,
        seven,
        version,
    })
}

fn leitura_gravada(value: Option<&Value>) -> Option<Leitura> {
    let map = value?.as_object()?;
    let mut remapped = Map::new();
    if let Some(v) = map.get("pct") {
        remapped.insert("used_percentage".into(), v.clone());
    }
    if let Some(v) = map.get("resetsAt") {
        remapped.insert("resets_at".into(), v.clone());
    }
    leitura(Some(&Value::Object(remapped)))
}

/// `amostrasDe` (`limites.ts:64`): a linha que não é uma amostra válida é ignorada, o resto segue em ordem de horário.
pub fn amostras_de(text: &str) -> Vec<Amostra> {
    let mut found = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            continue;
        }
        let Ok(doc) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(map) = doc.as_object() else {
            continue;
        };
        let Some(at) = map.get("at").and_then(Value::as_str) else {
            continue;
        };
        if parse_ts_ms(at).is_nan() {
            continue;
        }
        let five = leitura_gravada(map.get("five"));
        let seven = leitura_gravada(map.get("seven"));
        if five.is_none() && seven.is_none() {
            continue;
        }
        found.push(Amostra {
            at: at.to_owned(),
            five,
            seven,
            version: map
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    found.sort_by(|a, b| parse_ts_ms(&a.at).total_cmp(&parse_ts_ms(&b.at)));
    found
}

fn mesma_leitura(a: Option<Leitura>, b: Option<Leitura>) -> bool {
    a.map(|l| (l.pct, l.resets_at)) == b.map(|l| (l.pct, l.resets_at))
}

/// `vale` (`limites.ts:92`): a mesma leitura das duas janelas, ou uma leitura antes de `gap_s` da anterior, não vira
/// linha.
pub fn vale(nova: &Amostra, ultima: Option<&Amostra>, gap_s: f64) -> bool {
    let Some(ultima) = ultima else {
        return true;
    };
    if mesma_leitura(nova.five, ultima.five) && mesma_leitura(nova.seven, ultima.seven) {
        return false;
    }
    (parse_ts_ms(&nova.at) - parse_ts_ms(&ultima.at)) / 1000.0 >= gap_s
}

/// `dentroDaRetencao` (`limites.ts:98`).
pub fn dentro_da_retencao(amostras: &[Amostra], now_epoch_ms: f64, dias: f64) -> Vec<Amostra> {
    let corte = now_epoch_ms - dias * MS_POR_DIA;
    amostras
        .iter()
        .filter(|a| parse_ts_ms(&a.at) >= corte)
        .cloned()
        .collect()
}

/// `Atual` (`limites.ts:103`): a leitura e quando foi medida (ISO, UTC).
#[derive(Clone, Debug, PartialEq)]
pub struct Atual {
    pub pct: f64,
    pub resets_at: f64,
    pub at: String,
}

impl Atual {
    /// `{pct, resetsAt, at}`, a ordem do `{...leitura, at}` do TS.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("pct".into(), num(self.pct));
        map.insert("resetsAt".into(), num(self.resets_at));
        map.insert("at".into(), Value::String(self.at.clone()));
        Value::Object(map)
    }
}

/// `Partial<Record<Janela, Atual>>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atuais {
    pub five: Option<Atual>,
    pub seven: Option<Atual>,
}

impl Atuais {
    pub fn get(&self, janela: Janela) -> Option<&Atual> {
        match janela {
            Janela::Five => self.five.as_ref(),
            Janela::Seven => self.seven.as_ref(),
        }
    }

    fn set(&mut self, janela: Janela, atual: Atual) {
        match janela {
            Janela::Five => self.five = Some(atual),
            Janela::Seven => self.seven = Some(atual),
        }
    }
}

fn num(x: f64) -> Value {
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

/// `atuais` (`limites.ts:109`): a última leitura de cada janela ainda aberta (a renovação vem depois de `now`).
pub fn atuais(amostras: &[Amostra], now_epoch_ms: f64) -> Atuais {
    let agora = now_epoch_ms / 1000.0;
    let mut out = Atuais::default();
    for amostra in amostras {
        for janela in JANELAS {
            if let Some(l) = amostra.janela(janela)
                && l.resets_at > agora
            {
                out.set(
                    janela,
                    Atual {
                        pct: l.pct,
                        resets_at: l.resets_at,
                        at: amostra.at.clone(),
                    },
                );
            }
        }
    }
    out
}

/// `mtok` (`limites.ts:124`): milhões de tokens ponderados; leitura de cache pesa `cache_read_weight` de um token novo.
pub fn mtok(tok: &Tok, cache_read_weight: f64) -> f64 {
    (tok[0] + tok[1] + tok[2] + tok[3] * cache_read_weight) / TOKENS_POR_MILHAO
}

/// `Intervalo` (`limites.ts:128`).
#[derive(Clone, Debug, PartialEq)]
pub struct Intervalo {
    pub d_pct: f64,
    pub usd: f64,
    /// Milhões de tokens ponderados por modelo.
    pub mtok: IndexMap<String, f64>,
}

/// `intervalos` (`limites.ts:137`): da âncora até a primeira amostra em que o % subiu `min_delta_pct`; janela que
/// renovou reinicia a âncora.
pub fn intervalos(
    amostras: &[Amostra],
    janela: Janela,
    reqs: &[Req],
    opcoes: &Opcoes,
) -> Vec<Intervalo> {
    let lidas: Vec<(f64, Leitura)> = amostras
        .iter()
        .filter_map(|a| a.janela(janela).map(|l| (parse_ts_ms(&a.at) / 1000.0, l)))
        .collect();
    let mut ordenadas: Vec<&Req> = reqs.iter().collect();
    ordenadas.sort_by_key(|r| r.epoch_s);
    let mut out = Vec::new();
    let Some(&first) = lidas.first() else {
        return out;
    };
    let mut ancora = first;
    for &atual in &lidas[1..] {
        let renovou =
            (atual.1.resets_at - ancora.1.resets_at).abs() > SEGUNDOS_DE_FOLGA_DA_RENOVACAO;
        let delta = atual.1.pct - ancora.1.pct;
        if renovou || delta < 0.0 {
            ancora = atual;
            continue;
        }
        if delta < opcoes.min_delta_pct {
            continue;
        }
        let mut por_modelo: IndexMap<String, f64> = IndexMap::new();
        let mut usd = 0.0;
        for r in ordenadas
            .iter()
            .filter(|r| (r.epoch_s as f64) > ancora.0 && (r.epoch_s as f64) <= atual.0)
        {
            *por_modelo.entry(r.model.clone()).or_insert(0.0) +=
                mtok(&r.tok, opcoes.cache_read_weight);
            usd += r.cost;
        }
        if usd > 0.0 {
            out.push(Intervalo {
                d_pct: delta,
                usd,
                mtok: por_modelo,
            });
        }
        ancora = atual;
    }
    out
}

/// `ajusta` (`limites.ts:167`): mínimos quadrados com taxa ≥ 0 por modelo, descida por coordenada.
fn ajusta(itens: &[Intervalo], modelos: &[String]) -> IndexMap<String, f64> {
    let mut taxa: IndexMap<String, f64> = modelos.iter().map(|m| (m.clone(), 0.0)).collect();
    for _ in 0..PASSADAS_DO_AJUSTE {
        for m in modelos {
            let mut numerador = 0.0;
            let mut den = 0.0;
            for item in itens {
                let x = item.mtok.get(m).copied().unwrap_or(0.0);
                if x == 0.0 {
                    continue;
                }
                let resto = modelos.iter().fold(0.0, |acc, k| {
                    if k == m {
                        acc
                    } else {
                        acc + taxa[k] * item.mtok.get(k).copied().unwrap_or(0.0)
                    }
                });
                numerador += x * (item.d_pct - resto);
                den += x * x;
            }
            if den > 0.0 {
                taxa.insert(m.clone(), (numerador / den).max(0.0));
            }
        }
    }
    taxa
}

/// `Taxas` (`limites.ts:187`).
#[derive(Clone, Debug, PartialEq)]
pub struct Taxas {
    /// % da janela por milhão de tokens ponderados, por modelo calibrado.
    pub por_modelo: IndexMap<String, f64>,
    /// % da janela por US$ equivalente de API (para o plano, que só tem US$).
    pub por_usd: Option<f64>,
    pub intervalos: usize,
}

impl Taxas {
    /// `{porModelo, porUsd, intervalos}`.
    pub fn to_value(&self) -> Value {
        let mut modelos = Map::new();
        for (k, v) in &self.por_modelo {
            modelos.insert(k.clone(), num(*v));
        }
        let mut map = Map::new();
        map.insert("porModelo".into(), Value::Object(modelos));
        map.insert("porUsd".into(), self.por_usd.map_or(Value::Null, num));
        map.insert("intervalos".into(), Value::from(self.intervalos));
        Value::Object(map)
    }
}

/// `Calibracao` (`limites.ts:196`).
#[derive(Clone, Debug, PartialEq)]
pub struct Calibracao {
    pub five: Taxas,
    pub seven: Taxas,
}

impl Calibracao {
    pub fn get(&self, janela: Janela) -> &Taxas {
        match janela {
            Janela::Five => &self.five,
            Janela::Seven => &self.seven,
        }
    }

    /// `{five, seven}`.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("five".into(), self.five.to_value());
        map.insert("seven".into(), self.seven.to_value());
        Value::Object(map)
    }
}

/// `calibra` (`limites.ts:198`).
pub fn calibra(amostras: &[Amostra], reqs: &[Req], opcoes: &Opcoes) -> Calibracao {
    let fazer = |janela: Janela| -> Taxas {
        let itens = intervalos(amostras, janela, reqs, opcoes);
        if (itens.len() as f64) < opcoes.min_intervals {
            return Taxas {
                por_modelo: IndexMap::new(),
                por_usd: None,
                intervalos: itens.len(),
            };
        }
        let mut vistos: Vec<String> = Vec::new();
        for item in &itens {
            for m in item.mtok.keys() {
                if !vistos.contains(m) {
                    vistos.push(m.clone());
                }
            }
        }
        let candidatos: Vec<String> = vistos
            .into_iter()
            .filter(|m| {
                let n = itens
                    .iter()
                    .filter(|i| i.mtok.get(m).copied().unwrap_or(0.0) > 0.0)
                    .count();
                (n as f64) >= opcoes.min_intervals
            })
            .collect();
        let taxa = ajusta(&itens, &candidatos);
        let por_modelo: IndexMap<String, f64> =
            taxa.into_iter().filter(|(_, v)| *v > 0.0).collect();
        let usd = itens.iter().fold(0.0, |acc, i| acc + i.usd);
        let por_usd = (usd > 0.0).then(|| itens.iter().fold(0.0, |acc, i| acc + i.d_pct) / usd);
        Taxas {
            por_modelo,
            por_usd,
            intervalos: itens.len(),
        }
    };
    Calibracao {
        five: fazer(Janela::Five),
        seven: fazer(Janela::Seven),
    }
}

/// `Estimativa` (`limites.ts:222`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimativa {
    /// `None` = sem calibração para nenhum modelo dos tokens.
    pub pct: Option<f64>,
    /// Havia tokens de modelo sem taxa: o valor é só o que a calibração cobre.
    pub parcial: bool,
}

/// `pctDeTokens` (`limites.ts:230`): % da janela dos tokens por modelo, pelas taxas calibradas.
pub fn pct_de_tokens(
    taxas: &Taxas,
    tok_por_modelo: Option<&IndexMap<String, Tok>>,
    cache_read_weight: f64,
) -> Estimativa {
    let mut pct = 0.0;
    let mut coberto = false;
    let mut parcial = false;
    if let Some(mapa) = tok_por_modelo {
        for (modelo, tok) in mapa {
            let Some(taxa) = taxas.por_modelo.get(modelo) else {
                parcial = true;
                continue;
            };
            coberto = true;
            pct += taxa * mtok(tok, cache_read_weight);
        }
    }
    Estimativa {
        pct: coberto.then_some(pct),
        parcial,
    }
}

/// `Faixa` (`limites.ts:249`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Faixa {
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
}

impl Faixa {
    /// `{p25, median, p75}`.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("p25".into(), num(self.p25));
        map.insert("median".into(), num(self.median));
        map.insert("p75".into(), num(self.p75));
        Value::Object(map)
    }
}

/// `pctDeUsd` (`limites.ts:256`): US$ → % da janela pela taxa média de US$; sem taxa, sem faixa.
pub fn pct_de_usd(taxas: &Taxas, usd: Option<Faixa>) -> Option<Faixa> {
    let usd = usd?;
    let por_usd = taxas.por_usd?;
    Some(Faixa {
        p25: usd.p25 * por_usd,
        median: usd.median * por_usd,
        p75: usd.p75 * por_usd,
    })
}

/// `Cabe` (`limites.ts:262`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cabe {
    /// % que resta da janela atual; `None` = janela sem medida atual.
    pub resta: Option<f64>,
    /// `None` = não dá para dizer (sem medida ou sem calibração).
    pub cabe: Option<bool>,
}

/// `cabeNaJanela` (`limites.ts:270`): cabe quando o p75 não passa do que resta (100 − % usado) da janela atual.
pub fn cabe_na_janela(atual: Option<&Atual>, p75: Option<f64>) -> Cabe {
    let Some(atual) = atual else {
        return Cabe {
            resta: None,
            cabe: None,
        };
    };
    let resta = (PORCENTO - atual.pct).max(0.0);
    Cabe {
        resta: Some(resta),
        cabe: p75.map(|p| p <= resta),
    }
}

/// `somaFaixas` (`limites.ts:276`).
pub fn soma_faixas(faixas: &[Faixa]) -> Option<Faixa> {
    if faixas.is_empty() {
        return None;
    }
    Some(faixas.iter().fold(
        Faixa {
            p25: 0.0,
            median: 0.0,
            p75: 0.0,
        },
        |acc, f| Faixa {
            p25: acc.p25 + f.p25,
            median: acc.median + f.median,
            p75: acc.p75 + f.p75,
        },
    ))
}

/// Unidade do plano com o custo estimado em US$ (`{unit, costUsd}` de `limitesDoPlano`).
#[derive(Clone, Debug, PartialEq)]
pub struct UnidadeCusto {
    pub unit: String,
    pub cost_usd: Option<Faixa>,
}

/// `LimiteDoPlano` (`limites.ts:284`).
#[derive(Clone, Debug, PartialEq)]
pub struct LimiteDoPlano {
    /// Faixa em % da janela por unidade (`None` = unidade sem custo estimado, ou janela sem calibração).
    pub por_unidade: IndexMap<String, Option<Faixa>>,
    /// Soma das unidades com faixa; `None` sem nenhuma.
    pub plano: Option<Faixa>,
    /// Todas as unidades entraram na soma.
    pub completo: bool,
    pub atual: Option<Atual>,
    pub resta: Option<f64>,
    pub cabe: Option<bool>,
}

impl LimiteDoPlano {
    /// `{porUnidade, plano, completo, atual, resta, cabe}`.
    pub fn to_value(&self) -> Value {
        let mut unidades = Map::new();
        for (k, v) in &self.por_unidade {
            unidades.insert(k.clone(), v.as_ref().map_or(Value::Null, Faixa::to_value));
        }
        let mut map = Map::new();
        map.insert("porUnidade".into(), Value::Object(unidades));
        map.insert(
            "plano".into(),
            self.plano.as_ref().map_or(Value::Null, Faixa::to_value),
        );
        map.insert("completo".into(), Value::Bool(self.completo));
        map.insert(
            "atual".into(),
            self.atual.as_ref().map_or(Value::Null, Atual::to_value),
        );
        map.insert("resta".into(), self.resta.map_or(Value::Null, num));
        map.insert("cabe".into(), self.cabe.map_or(Value::Null, Value::Bool));
        Value::Object(map)
    }
}

/// `LimiteDoPlano` das duas janelas.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitesDoPlano {
    pub five: LimiteDoPlano,
    pub seven: LimiteDoPlano,
}

impl LimitesDoPlano {
    pub fn get(&self, janela: Janela) -> &LimiteDoPlano {
        match janela {
            Janela::Five => &self.five,
            Janela::Seven => &self.seven,
        }
    }

    /// `{five, seven}`.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("five".into(), self.five.to_value());
        map.insert("seven".into(), self.seven.to_value());
        Value::Object(map)
    }
}

/// `limitesDoPlano` (`limites.ts:297`): faixa p25–p75 do plano em % de cada janela e se cabe no que resta da janela
/// atual.
pub fn limites_do_plano(
    unidades: &[UnidadeCusto],
    calibracao: &Calibracao,
    atual: &Atuais,
) -> LimitesDoPlano {
    let fazer = |janela: Janela| -> LimiteDoPlano {
        let mut por_unidade: IndexMap<String, Option<Faixa>> = IndexMap::new();
        for u in unidades {
            por_unidade.insert(
                u.unit.clone(),
                pct_de_usd(calibracao.get(janela), u.cost_usd),
            );
        }
        let faixas: Vec<Faixa> = por_unidade.values().filter_map(|f| *f).collect();
        let plano = soma_faixas(&faixas);
        let completo = faixas.len() == unidades.len();
        let a = atual.get(janela);
        let Cabe { resta, cabe } = cabe_na_janela(a, plano.map(|p| p.p75));
        let cabe = if cabe == Some(true) && !completo {
            None
        } else {
            cabe
        };
        LimiteDoPlano {
            por_unidade,
            plano,
            completo,
            atual: a.cloned(),
            resta,
            cabe,
        }
    };
    LimitesDoPlano {
        five: fazer(Janela::Five),
        seven: fazer(Janela::Seven),
    }
}

/// `textoPct` (`limites.ts:289`): `~12.3%` (estimado), `~12.3%+` (parcial) ou `sem calib.`.
pub fn texto_pct(e: &Estimativa) -> String {
    match e.pct {
        None => "sem calib.".to_owned(),
        Some(pct) => format!("~{}%{}", to_fixed(pct, 1), if e.parcial { "+" } else { "" }),
    }
}

/// `Chaves` (`limites-io.ts:21`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chaves {
    pub opcoes: Opcoes,
    pub gap_s: f64,
    pub retention_days: f64,
}

/// `inteiro` (`limites-io.ts:24`): valor fora da faixa só avisa e usa o padrão.
fn inteiro(env: &Env, cwd: &Path, key: &str, fallback: u32, min: u32, max: u32) -> f64 {
    let raw = config_value(env, cwd, key, &fallback.to_string());
    if let Some(value) = js_number(&raw)
        && value.is_finite()
        && value.fract() == 0.0
        && value >= f64::from(min)
        && value <= f64::from(max)
    {
        return value;
    }
    warn(&format!(
        "limites: {key}={raw}: esperava um inteiro de {min} a {max}; uso {fallback}"
    ));
    f64::from(fallback)
}

/// `chaves` (`limites-io.ts:32`).
pub fn chaves(env: &Env, cwd: &Path) -> Chaves {
    Chaves {
        gap_s: inteiro(env, cwd, "HADES_LIMITS_SAMPLE_GAP_S", 60, 0, 3600),
        retention_days: inteiro(env, cwd, "HADES_LIMITS_RETENTION_DAYS", 30, 1, 365),
        opcoes: Opcoes {
            min_delta_pct: inteiro(env, cwd, "HADES_LIMITS_MIN_DELTA_PCT", 1, 1, 50),
            min_intervals: inteiro(env, cwd, "HADES_LIMITS_MIN_INTERVALS", 5, 2, 100),
            cache_read_weight: inteiro(env, cwd, "HADES_LIMITS_CACHE_READ_WEIGHT_PCT", 10, 0, 100)
                / PORCENTO,
        },
    }
}

/// `lerAmostras` (`limites-io.ts:59`): `<hades_home>/limits/samples.jsonl`; sem o arquivo, nenhuma amostra e nenhuma
/// pasta criada.
pub fn ler_amostras(env: &Env) -> Vec<Amostra> {
    let file = hades_home(env).join("limits").join("samples.jsonl");
    amostras_de(&read_text(&file).unwrap_or_default())
}

/// `Medidas` de `medidas` (`limites-io.ts:79`).
#[derive(Clone, Debug, PartialEq)]
pub struct Medidas {
    pub amostras: Vec<Amostra>,
    pub atual: Atuais,
}

/// `medidas` (`limites-io.ts:79`): amostras dentro da retenção e a leitura atual de cada janela.
pub fn medidas(env: &Env, now_epoch_ms: f64, k: &Chaves) -> Medidas {
    let amostras = dentro_da_retencao(&ler_amostras(env), now_epoch_ms, k.retention_days);
    let atual = atuais(&amostras, now_epoch_ms);
    Medidas { amostras, atual }
}

fn hora(epoch_s: f64) -> String {
    iso_from_epoch(epoch_s.floor() as i64)
}

/// `renova` (`limites-io.ts:85`): hora do dia na janela de 5 h; data e hora na semanal (sempre UTC).
pub fn renova(janela: Janela, epoch_s: f64) -> String {
    let iso = hora(epoch_s);
    match janela {
        Janela::Five => format!("{}Z", &iso[11..16]),
        Janela::Seven => format!("{} {}Z", &iso[5..10], &iso[11..16]),
    }
}

/// `Math.round` do JS: meio sobe.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `linhaAtual` (`limites-io.ts:91`): `5 h 23% (renova 22:10Z) · semana 41% (renova 11-07 03:00Z)`.
pub fn linha_atual(atual: &Atuais) -> String {
    JANELAS
        .iter()
        .filter_map(|&j| {
            atual.get(j).map(|a| {
                format!(
                    "{} {}% (renova {})",
                    j.rotulo(),
                    js_round(a.pct),
                    renova(j, a.resets_at)
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `LimitsNow` (`usage.ts:324`).
#[derive(Clone, Debug, PartialEq)]
pub struct LimitsNow {
    pub k: Chaves,
    pub atual: Atuais,
    pub calibracao: Calibracao,
    pub amostras: usize,
}

fn mtime_day(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(iso_from_epoch(secs)[..10].to_owned())
}

type ChaveDoArquivo = (u64, i128, u64);
type CacheDeRequisicoes = HashMap<PathBuf, (ChaveDoArquivo, Vec<Req>)>;

/// Cache em memória das requisições por transcript, chaveado por tamanho, mtime e `since_ms`: o app relê `limits_now`
/// a cada ~30 s e, sem isto, reparseia GBs de transcripts a cada leitura. Só o que mudou desde a última leitura
/// (a sessão ativa) volta a ser parseado. Extensão do Rust: o TS roda uma vez por processo e não precisa disto.
static CACHE_REQS: Mutex<Option<CacheDeRequisicoes>> = Mutex::new(None);

fn chave_do_arquivo(path: &Path, since_ms: f64) -> Option<ChaveDoArquivo> {
    let meta = fs::metadata(path).ok()?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    Some((meta.len(), mtime_ns, since_ms.to_bits()))
}

fn requisicoes_do_arquivo(path: &Path, rules: &GuardRules, since_ms: f64) -> Vec<Req> {
    let Some(chave) = chave_do_arquivo(path, since_ms) else {
        return parse_file_full(path, rules, since_ms).1;
    };
    {
        let cache = CACHE_REQS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((k, reqs)) = cache.as_ref().and_then(|m| m.get(path))
            && *k == chave
        {
            return reqs.clone();
        }
    }
    let reqs = parse_file_full(path, rules, since_ms).1;
    CACHE_REQS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), (chave, reqs.clone()));
    reqs
}

/// `limitsNow` (`usage.ts:332`): medida atual e calibração para quem não lista consumo (`moirai estimate`); sem
/// amostra não lê transcript. Não grava `usage/cache.json`: relê os transcripts do dia da amostra mais antiga.
pub fn limits_now(env: &Env, cwd: &Path) -> Result<LimitsNow, HadesError> {
    let k = chaves(env, cwd);
    let agora = crate::history::clock_ms(env)? as f64;
    let Medidas { amostras, atual } = medidas(env, agora, &k);
    let Some(oldest) = amostras.first() else {
        return Ok(LimitsNow {
            k,
            atual,
            calibracao: calibra(&[], &[], &k.opcoes),
            amostras: 0,
        });
    };
    let load_since: String = oldest.at.chars().take(10).collect();
    let since_ms = parse_ts_ms(&format!("{load_since}T00:00:00Z"));
    let rules = GuardRules::load(env).map_err(HadesError::new)?;
    let projects = env.home().join(".claude").join("projects");
    let mut reqs: Vec<Req> = Vec::new();
    for path in transcripts_in(&projects) {
        let Some(day) = mtime_day(&path) else {
            continue;
        };
        if day < load_since {
            continue;
        }
        reqs.extend(requisicoes_do_arquivo(&path, &rules, since_ms));
    }
    let calibracao = calibra(&amostras, &reqs, &k.opcoes);
    let total = amostras.len();
    Ok(LimitsNow {
        k,
        atual,
        calibracao,
        amostras: total,
    })
}

#[cfg(test)]
mod tests;
