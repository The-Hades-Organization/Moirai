//! Estimativa por unidade (`src/lib/moirai.ts:305-333`).

use std::collections::HashMap;

use crate::model::{Basis, Confidence, HistoryUnit, Mode, Params, PlanUnit, UnitEstimate};
use crate::stats::{Summary, band_of, by_size, confidence_of, declared_lines, summarize};

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `estimateUnit` (`src/lib/moirai.ts:305`): parecidas (papel, modo e faixa), depois só tamanho, depois o histórico
/// geral, depois nada.
pub fn estimate_unit(
    unit: &PlanUnit,
    mode: Mode,
    history: &[HistoryUnit],
    params: &Params,
    size_hint: Option<f64>,
) -> UnitEstimate {
    let lines = declared_lines(&unit.scope).or(size_hint);
    let band = lines.map(|l| band_of(l, params));
    let build = |basis: Basis, sample: usize, confidence: Confidence, s: Summary, note: String| {
        UnitEstimate {
            unit: unit.name.clone(),
            role: unit.role.clone(),
            mode,
            declared_lines: lines,
            band,
            basis,
            sample,
            confidence,
            cost_usd: s.cost_usd,
            wall_min: s.wall_min,
            gate_min: s.gate_min,
            tokens: s.tokens,
            note,
        }
    };
    let similar: Vec<&HistoryUnit> = history
        .iter()
        .filter(|u| u.role == unit.role && u.mode == mode && (band.is_none() || u.band == band))
        .collect();
    let band_text = band.map_or(String::new(), |b| format!(", {}", b.as_str()));
    if similar.len() as f64 >= params.min_sample {
        let found = confidence_of(similar.len(), params);
        let confidence = if band.is_none() && found == Confidence::Alta {
            Confidence::Media
        } else {
            found
        };
        let size_note = if band.is_none() {
            "; escopo sem tamanho declarado (use \"~N linhas\" ou \"N arquivos\")"
        } else {
            ""
        };
        let note = format!(
            "{} parecidas ({}, {}{band_text}){size_note}",
            similar.len(),
            unit.role,
            mode.as_str()
        );
        return build(
            Basis::Parecidas,
            similar.len(),
            confidence,
            summarize(&similar),
            note,
        );
    }
    let why = format!(
        "só {} parecida{} ({}, {}{band_text}), mínimo {}",
        similar.len(),
        plural(similar.len()),
        unit.role,
        mode.as_str(),
        hades_core::jsfmt::to_fixed(params.min_sample, 0)
    );
    if let Some(lines) = lines {
        let (n, size) = by_size(history, lines);
        if size.cost_usd.is_some() || size.wall_min.is_some() {
            let note = format!(
                "{why}; regra só de tamanho: {} linhas × taxa de {n} unidades",
                hades_core::jsfmt::to_fixed(lines, 0)
            );
            return build(Basis::Tamanho, n, Confidence::Baixa, size, note);
        }
    }
    if !history.is_empty() {
        let all: Vec<&HistoryUnit> = history.iter().collect();
        let note = format!(
            "{why}; sem tamanho declarado no escopo: mediana de todo o histórico ({} unidades)",
            history.len()
        );
        return build(
            Basis::Geral,
            history.len(),
            Confidence::Baixa,
            summarize(&all),
            note,
        );
    }
    build(
        Basis::SemHistorico,
        0,
        Confidence::Nenhuma,
        Summary::default(),
        "sem histórico de unidades encerradas".to_owned(),
    )
}

/// `estimatePlan` (`src/lib/moirai.ts:329`).
pub fn estimate_plan(
    units: &[PlanUnit],
    mode: Mode,
    history: &[HistoryUnit],
    params: &Params,
    size_hints: &HashMap<String, f64>,
) -> Vec<UnitEstimate> {
    units
        .iter()
        .map(|u| estimate_unit(u, mode, history, params, size_hints.get(&u.name).copied()))
        .collect()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::model::{HistoryUnit, Mode};

    pub fn unit() -> HistoryUnit {
        HistoryUnit {
            name: "u".into(),
            root: "r".into(),
            mode: Mode::Skill,
            role: "impl".into(),
            state: "done".into(),
            created_at: "2026-10-01T00:00:00Z".into(),
            done_at: "2026-10-01T01:00:00Z".into(),
            cost_usd: Some(1.0),
            tokens: Some(1000.0),
            wall_s: Some(3600.0),
            gate_s: 0.0,
            gate_peak_mb: None,
            clears: 0.0,
            resumes: None,
            files: None,
            lines: None,
            band: None,
        }
    }

    pub fn many(n: usize, over: impl Fn(usize, &mut HistoryUnit)) -> Vec<HistoryUnit> {
        (0..n)
            .map(|i| {
                let mut u = unit();
                u.name = format!("u{i}");
                over(i, &mut u);
                u
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{many, unit};
    use super::*;
    use crate::model::Band;

    const PARAMS: Params = Params {
        min_sample: 5.0,
        high_factor: 3.0,
        small_lines: 150.0,
        large_lines: 600.0,
    };

    fn plan_unit(scope: &str) -> PlanUnit {
        PlanUnit {
            name: "n".into(),
            role: "impl".into(),
            scope: scope.into(),
        }
    }

    fn est(scope: &str, history: &[HistoryUnit]) -> UnitEstimate {
        estimate_unit(&plan_unit(scope), Mode::Skill, history, &PARAMS, None)
    }

    #[test]
    fn dado_7_parecidas_entao_mediana_e_faixa_vem_delas_e_a_confianca_e_media() {
        let history = many(7, |i, u| {
            u.cost_usd = Some(i as f64 + 1.0);
            u.wall_s = Some((i as f64 + 1.0) * 600.0);
        });
        let e = est("sem número", &history);
        assert_eq!(
            (e.basis, e.sample, e.confidence),
            (Basis::Parecidas, 7, Confidence::Media)
        );
        let c = e.cost_usd.unwrap();
        assert_eq!((c.p25, c.median, c.p75), (2.5, 4.0, 5.5));
        assert_eq!(e.wall_min.unwrap().median, 40.0);
    }

    #[test]
    fn dado_tamanho_declarado_e_15_parecidas_da_mesma_faixa_entao_a_confianca_e_alta() {
        let history = many(15, |_, u| {
            u.lines = Some(300.0);
            u.band = Some(Band::Media);
        });
        assert_eq!(est("~300 linhas", &history).confidence, Confidence::Alta);
    }

    #[test]
    fn dado_papel_e_modo_diferentes_entao_as_unidades_nao_sao_parecidas() {
        let history = many(9, |_, u| {
            u.role = "spec".into();
            u.mode = Mode::Projeto;
        });
        assert_eq!(est("", &history).basis, Basis::Geral);
    }

    #[test]
    fn dado_menos_de_5_parecidas_e_tamanho_declarado_entao_cai_para_a_regra_so_de_tamanho() {
        let mut history = many(6, |_, u| {
            u.role = "spec".into();
            u.cost_usd = Some(10.0);
            u.lines = Some(100.0);
            u.wall_s = Some(600.0);
            u.band = Some(Band::Pequena);
        });
        let mut extra = unit();
        extra.lines = Some(100.0);
        extra.cost_usd = Some(10.0);
        history.push(extra);
        let e = est("~200 linhas", &history);
        assert_eq!((e.basis, e.confidence), (Basis::Tamanho, Confidence::Baixa));
        assert_eq!(e.cost_usd.unwrap().median, 20.0);
        assert!(e.note.contains("só 0 parecidas"), "{}", e.note);
    }

    #[test]
    fn dado_exatamente_4_parecidas_entao_ainda_e_baixa_e_com_5_deixa_de_ser() {
        assert_eq!(est("", &many(4, |_, _| {})).confidence, Confidence::Baixa);
        assert_eq!(est("", &many(5, |_, _| {})).confidence, Confidence::Media);
    }

    #[test]
    fn dado_menos_de_5_parecidas_e_escopo_sem_tamanho_entao_usa_a_mediana_geral_e_avisa() {
        let e = est("", &many(3, |_, _| {}));
        assert_eq!((e.basis, e.confidence), (Basis::Geral, Confidence::Baixa));
        assert!(e.note.contains("sem tamanho declarado"));
    }

    #[test]
    fn dado_historico_vazio_entao_sem_estimativa_e_confianca_nenhuma() {
        let e = est("~10 linhas", &[]);
        assert_eq!(
            (e.basis, e.confidence, e.cost_usd),
            (Basis::SemHistorico, Confidence::Nenhuma, None)
        );
    }

    #[test]
    fn dado_tempo_de_gate_zero_em_todas_entao_o_gate_nao_vira_mediana_zero() {
        assert_eq!(est("", &many(6, |_, u| u.gate_s = 0.0)).gate_min, None);
    }

    #[test]
    fn dado_dica_do_deep_e_escopo_sem_tamanho_entao_a_faixa_de_tamanho_vem_da_dica() {
        let history = many(6, |_, u| {
            u.lines = Some(700.0);
            u.band = Some(Band::Grande);
        });
        let hints = HashMap::from([("n".to_owned(), 800.0)]);
        let [e] = estimate_plan(&[plan_unit("")], Mode::Skill, &history, &PARAMS, &hints)
            .try_into()
            .unwrap();
        assert_eq!((e.band, e.basis), (Some(Band::Grande), Basis::Parecidas));
    }
}
