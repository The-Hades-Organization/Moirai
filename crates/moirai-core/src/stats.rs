//! Estatística e tamanho da estimativa (`src/lib/moirai.ts:81-105, 265-303`).

use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Band, Confidence, HistoryUnit, Params, Spread};

/// `LINES_PER_FILE` (`src/lib/moirai.ts:11`): linhas por arquivo quando o escopo declara só arquivos.
pub const LINES_PER_FILE: f64 = 60.0;

/// `MIN_PER_S` (`src/lib/moirai.ts:265`).
pub const MIN_PER_S: f64 = 1.0 / 60.0;

/// Resumo de custo, tempo e tokens de um conjunto de unidades.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Summary {
    pub cost_usd: Option<Spread>,
    pub wall_min: Option<Spread>,
    pub gate_min: Option<Spread>,
    pub tokens: Option<Spread>,
}

/// `quantile` (`src/lib/moirai.ts:81`): interpolação linear entre vizinhos (método 7); a lista vem ordenada e não vazia.
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = (sorted.len() - 1) as f64 * q;
    let low = at.floor();
    let high = at.ceil();
    sorted[low as usize] + (sorted[high as usize] - sorted[low as usize]) * (at - low)
}

/// `spread` (`src/lib/moirai.ts:88`): nulos e não finitos ficam de fora; sem valor, sem faixa.
pub fn spread(values: &[Option<f64>]) -> Option<Spread> {
    let mut sorted: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(Spread {
        n: sorted.len(),
        p25: quantile(&sorted, 0.25),
        median: quantile(&sorted, 0.5),
        p75: quantile(&sorted, 0.75),
    })
}

/// `scaled` (`src/lib/moirai.ts:94`).
pub fn scaled(s: Option<Spread>, factor: f64) -> Option<Spread> {
    s.map(|s| Spread {
        n: s.n,
        p25: s.p25 * factor,
        median: s.median * factor,
        p75: s.p75 * factor,
    })
}

/// `bandOf` (`src/lib/moirai.ts:99`).
pub fn band_of(lines: f64, params: &Params) -> Band {
    if lines < params.small_lines {
        return Band::Pequena;
    }
    if lines >= params.large_lines {
        Band::Grande
    } else {
        Band::Media
    }
}

static DECLARED_LINES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([0-9][0-9.]*)\s*linhas").expect("regex fixa"));
static DECLARED_FILES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([0-9]+)\s*arquivos").expect("regex fixa"));

/// `declaredLines` (`src/lib/moirai.ts:105`): "~300 linhas" ou "5 arquivos"; `None` se o escopo não declara tamanho.
pub fn declared_lines(scope: &str) -> Option<f64> {
    if let Some(m) = DECLARED_LINES.captures(scope) {
        return Some(m[1].replace('.', "").parse::<f64>().unwrap_or(f64::NAN));
    }
    DECLARED_FILES
        .captures(scope)
        .map(|m| m[1].parse::<f64>().unwrap_or(f64::NAN) * LINES_PER_FILE)
}

/// `gateOrNull` (`src/lib/moirai.ts:268`): unidade que nunca passou pelo gate não informa tempo de gate.
fn gate_or_none(u: &HistoryUnit) -> Option<f64> {
    (u.gate_s > 0.0).then_some(u.gate_s)
}

/// `summarize` (`src/lib/moirai.ts:270`).
pub fn summarize(units: &[&HistoryUnit]) -> Summary {
    let pick = |f: &dyn Fn(&HistoryUnit) -> Option<f64>| -> Vec<Option<f64>> {
        units.iter().map(|u| f(u)).collect()
    };
    Summary {
        cost_usd: spread(&pick(&|u| u.cost_usd)),
        wall_min: scaled(spread(&pick(&|u| u.wall_s)), MIN_PER_S),
        gate_min: scaled(spread(&pick(&gate_or_none)), MIN_PER_S),
        tokens: spread(&pick(&|u| u.tokens)),
    }
}

/// `confidenceOf` (`src/lib/moirai.ts:278`).
pub fn confidence_of(n: usize, params: &Params) -> Confidence {
    let n = n as f64;
    if n < params.min_sample {
        return Confidence::Baixa;
    }
    if n >= params.min_sample * params.high_factor {
        Confidence::Alta
    } else {
        Confidence::Media
    }
}

/// `perLine` (`src/lib/moirai.ts:284`): mediana e quartis da taxa por linha de diff entre as unidades com tamanho real.
fn per_line(units: &[&HistoryUnit], pick: &dyn Fn(&HistoryUnit) -> Option<f64>) -> Option<Spread> {
    let rates: Vec<Option<f64>> = units
        .iter()
        .map(|u| match (pick(u), u.lines) {
            (Some(value), Some(lines)) if lines > 0.0 => Some(value / lines),
            _ => None,
        })
        .collect();
    spread(&rates)
}

/// `bySize` (`src/lib/moirai.ts:294`): taxa por linha do histórico inteiro × linhas declaradas. `n` conta as unidades
/// com tamanho real.
pub fn by_size(history: &[HistoryUnit], lines: f64) -> (usize, Summary) {
    let sized: Vec<&HistoryUnit> = history
        .iter()
        .filter(|u| u.lines.is_some_and(|l| l > 0.0))
        .collect();
    let summary = Summary {
        cost_usd: scaled(per_line(&sized, &|u| u.cost_usd), lines),
        wall_min: scaled(per_line(&sized, &|u| u.wall_s), lines * MIN_PER_S),
        gate_min: scaled(per_line(&sized, &gate_or_none), lines * MIN_PER_S),
        tokens: scaled(per_line(&sized, &|u| u.tokens), lines),
    };
    (sized.len(), summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS: Params = Params {
        min_sample: 5.0,
        high_factor: 3.0,
        small_lines: 150.0,
        large_lines: 600.0,
    };

    fn some(values: &[f64]) -> Vec<Option<f64>> {
        values.iter().copied().map(Some).collect()
    }

    #[test]
    fn dado_1_a_5_entao_mediana_3_p25_2_e_p75_4() {
        assert_eq!(
            spread(&some(&[5.0, 1.0, 3.0, 2.0, 4.0])),
            Some(Spread {
                n: 5,
                p25: 2.0,
                median: 3.0,
                p75: 4.0
            })
        );
    }

    #[test]
    fn dado_numero_par_entao_o_quantil_interpola_entre_os_vizinhos() {
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.25), 1.75);
    }

    #[test]
    fn dado_um_valor_so_entao_os_tres_quantis_sao_ele_e_vazio_ou_nulos_nao_geram_faixa() {
        assert_eq!(
            spread(&some(&[7.0])),
            Some(Spread {
                n: 1,
                p25: 7.0,
                median: 7.0,
                p75: 7.0
            })
        );
        assert_eq!(spread(&[]), None);
        assert_eq!(spread(&[None, None]), None);
    }

    #[test]
    fn dado_valor_nao_finito_entao_fica_fora_da_faixa() {
        let s = spread(&[Some(1.0), Some(f64::NAN), Some(f64::INFINITY), Some(3.0)]).unwrap();
        assert_eq!((s.n, s.median), (2, 2.0));
    }

    #[test]
    fn dado_o_escopo_entao_linhas_valem_direto_e_arquivos_valem_60_linhas_cada() {
        assert_eq!(declared_lines("mexe em ~300 linhas"), Some(300.0));
        assert_eq!(declared_lines("uns 5 arquivos"), Some(300.0));
        assert_eq!(declared_lines("sem número"), None);
    }

    #[test]
    fn dado_milhar_com_ponto_e_caixa_alta_entao_le_o_numero() {
        assert_eq!(declared_lines("~1.200 LINHAS"), Some(1200.0));
        assert_eq!(declared_lines("3 Arquivos"), Some(180.0));
        assert_eq!(declared_lines("50 linhas e 5 arquivos"), Some(50.0));
    }

    #[test]
    fn dado_o_limite_entao_149_e_pequena_150_media_600_grande() {
        assert_eq!(band_of(149.0, &PARAMS), Band::Pequena);
        assert_eq!(band_of(150.0, &PARAMS), Band::Media);
        assert_eq!(band_of(600.0, &PARAMS), Band::Grande);
    }

    #[test]
    fn dado_n_entao_a_confianca_muda_em_4_5_e_15() {
        assert_eq!(confidence_of(4, &PARAMS), Confidence::Baixa);
        assert_eq!(confidence_of(5, &PARAMS), Confidence::Media);
        assert_eq!(confidence_of(14, &PARAMS), Confidence::Media);
        assert_eq!(confidence_of(15, &PARAMS), Confidence::Alta);
    }
}
