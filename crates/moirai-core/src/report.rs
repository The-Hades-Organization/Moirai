//! Saída do `moirai estimate`: JSON e texto (`src/lib/moirai.ts:432`, `src/commands/moirai.ts:273-299`).

use hades_core::jsfmt::to_fixed;
use serde_json::{Map, Number, Value, json};

use crate::limits::{Faixa, JANELAS, LimitesDoPlano, UnidadeCusto, renova};
use crate::model::{Params, Spread, UnitEstimate};
use crate::plan::{TABLE_HEADER, estimate_row};

/// `JSON_VERSION` (`src/lib/moirai.ts:7`).
pub const JSON_VERSION: u32 = 1;

fn num(x: f64) -> Value {
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

fn spread_value(s: &Option<Spread>) -> Value {
    s.as_ref().map_or(
        Value::Null,
        |s| json!({ "n": s.n, "p25": num(s.p25), "median": num(s.median), "p75": num(s.p75) }),
    )
}

/// `UnitEstimate` na ordem de chaves do objeto do TS (`src/lib/moirai.ts:58`).
pub fn estimate_value(e: &UnitEstimate) -> Value {
    json!({
        "unit": e.unit,
        "role": e.role,
        "mode": e.mode.as_str(),
        "declaredLines": e.declared_lines.map_or(Value::Null, num),
        "band": e.band.map_or(Value::Null, |b| Value::String(b.as_str().to_owned())),
        "basis": e.basis.as_str(),
        "sample": e.sample,
        "confidence": e.confidence.as_str(),
        "costUsd": spread_value(&e.cost_usd),
        "wallMin": spread_value(&e.wall_min),
        "gateMin": spread_value(&e.gate_min),
        "tokens": spread_value(&e.tokens),
        "note": e.note,
    })
}

/// `estimateJson` + `limits` (`src/lib/moirai.ts:432`, `src/commands/moirai.ts:334`).
pub fn estimate_json_value(
    plan: &str,
    generated_at: &str,
    estimates: &[UnitEstimate],
    params: &Params,
    history_size: usize,
    limits: &LimitesDoPlano,
) -> Value {
    let mut map = Map::new();
    map.insert("v".into(), json!(JSON_VERSION));
    map.insert("plan".into(), json!(plan));
    map.insert("generatedAt".into(), json!(generated_at));
    map.insert(
        "params".into(),
        json!({ "minSample": num(params.min_sample) }),
    );
    map.insert("history".into(), json!({ "units": history_size }));
    map.insert(
        "units".into(),
        Value::Array(estimates.iter().map(estimate_value).collect()),
    );
    map.insert("limits".into(), limits.to_value());
    Value::Object(map)
}

/// Custo de cada unidade para o `limitesDoPlano` (`src/commands/moirai.ts:332`).
pub fn costs_of(estimates: &[UnitEstimate]) -> Vec<UnidadeCusto> {
    estimates
        .iter()
        .map(|e| UnidadeCusto {
            unit: e.unit.clone(),
            cost_usd: e.cost_usd.map(|s| Faixa {
                p25: s.p25,
                median: s.median,
                p75: s.p75,
            }),
        })
        .collect()
}

/// `faixaText` (`src/commands/moirai.ts:273`).
fn faixa_text(f: Option<&Faixa>) -> String {
    f.map_or_else(
        || "sem calib.".to_owned(),
        |f| format!("{}–{}%", to_fixed(f.p25, 1), to_fixed(f.p75, 1)),
    )
}

/// `cabeText` (`src/commands/moirai.ts:275`).
fn cabe_text(l: &crate::limits::LimiteDoPlano) -> &'static str {
    match l.cabe {
        None if l.atual.is_some() => "não dá para dizer",
        None => "sem medida atual",
        Some(true) => "cabe",
        Some(false) => "NÃO cabe",
    }
}

/// `limitsLines` (`src/commands/moirai.ts:280`).
fn limits_lines(estimates: &[UnitEstimate], limits: &LimitesDoPlano) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        "% das janelas (p25–p75, estimado pela calibração de tokens × % medido; reference/limites.md)".to_owned(),
        format!(
            "| unidade | {} | {} |",
            JANELAS[0].rotulo(),
            JANELAS[1].rotulo()
        ),
    ];
    for e in estimates {
        let cell = |l: &crate::limits::LimiteDoPlano| {
            faixa_text(l.por_unidade.get(&e.unit).and_then(Option::as_ref))
        };
        lines.push(format!(
            "| {} | {} | {} |",
            e.unit,
            cell(&limits.five),
            cell(&limits.seven)
        ));
    }
    for janela in JANELAS {
        let l = limits.get(janela);
        let resta = match (&l.atual, l.resta) {
            (Some(a), Some(r)) => format!(
                "restam {}% (renova {})",
                to_fixed(r, 1),
                renova(janela, a.resets_at)
            ),
            _ => "sem medida atual".to_owned(),
        };
        let parcial = if l.completo || l.plano.is_none() {
            ""
        } else {
            " (só as unidades com custo estimado)"
        };
        lines.push(format!(
            "janela de {}: plano {}{parcial} · {resta} · cabe na janela atual? {}",
            janela.rotulo(),
            faixa_text(l.plano.as_ref()),
            cabe_text(l)
        ));
    }
    lines
}

/// `textOf` (`src/commands/moirai.ts:293`).
pub fn estimate_text(
    plan: &str,
    estimates: &[UnitEstimate],
    limits: &LimitesDoPlano,
) -> Vec<String> {
    let mut lines = vec![
        format!("estimativa de {plan}"),
        format!("| {} |", TABLE_HEADER.join(" | ")),
    ];
    lines.extend(
        estimates
            .iter()
            .map(|e| format!("| {} |", estimate_row(e).join(" | "))),
    );
    for e in estimates {
        if matches!(
            e.confidence,
            crate::model::Confidence::Baixa | crate::model::Confidence::Nenhuma
        ) {
            lines.push(format!(
                "CONFIANÇA {} em {}: {}",
                e.confidence.as_str().to_uppercase(),
                e.unit,
                e.note
            ));
        }
    }
    lines.extend(limits_lines(estimates, limits));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimate::estimate_plan;
    use crate::limits::{Atuais, Calibracao, Taxas, limites_do_plano};
    use crate::model::{Mode, PlanUnit};
    use indexmap::IndexMap;
    use std::collections::HashMap;

    fn params() -> Params {
        Params {
            min_sample: 5.0,
            high_factor: 2.0,
            small_lines: 150.0,
            large_lines: 600.0,
        }
    }

    fn sem_calibracao() -> Calibracao {
        let vazia = || Taxas {
            por_modelo: IndexMap::new(),
            por_usd: None,
            intervalos: 0,
        };
        Calibracao {
            five: vazia(),
            seven: vazia(),
        }
    }

    fn sem_historico() -> (Vec<UnitEstimate>, LimitesDoPlano) {
        let units = [PlanUnit {
            name: "u-um".into(),
            role: "impl".into(),
            scope: "120 linhas".into(),
        }];
        let estimates = estimate_plan(&units, Mode::Projeto, &[], &params(), &HashMap::new());
        let limits = limites_do_plano(&costs_of(&estimates), &sem_calibracao(), &Atuais::default());
        (estimates, limits)
    }

    #[test]
    fn json_tem_as_chaves_na_ordem_do_ts() {
        let (estimates, limits) = sem_historico();
        let value = estimate_json_value(
            "/p.md",
            "2026-11-20T12:00:00.000Z",
            &estimates,
            &params(),
            0,
            &limits,
        );
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "v",
                "plan",
                "generatedAt",
                "params",
                "history",
                "units",
                "limits"
            ]
        );
        let unit_keys: Vec<&str> = value["units"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            unit_keys,
            [
                "unit",
                "role",
                "mode",
                "declaredLines",
                "band",
                "basis",
                "sample",
                "confidence",
                "costUsd",
                "wallMin",
                "gateMin",
                "tokens",
                "note"
            ]
        );
        assert_eq!(value["units"][0]["basis"], "sem-historico");
    }

    #[test]
    fn texto_sem_medida_e_sem_calibracao() {
        let (estimates, limits) = sem_historico();
        let text = estimate_text("/p.md", &estimates, &limits);
        assert_eq!(text[0], "estimativa de /p.md");
        assert!(text.contains(
            &"CONFIANÇA NENHUMA em u-um: sem histórico de unidades encerradas".to_owned()
        ));
        assert!(text.contains(&"| u-um | sem calib. | sem calib. |".to_owned()));
        assert_eq!(
            text.last().unwrap(),
            "janela de semana: plano sem calib. · sem medida atual · cabe na janela atual? sem medida atual"
        );
    }
}
