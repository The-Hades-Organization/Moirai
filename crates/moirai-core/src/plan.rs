//! Tabela de unidades e seção `## Estimativa` do plano (`src/lib/moirai.ts:225-266, 335-430, 498`).

use std::sync::LazyLock;

use regex::Regex;

use crate::jsnum::js_number;
use crate::model::{Params, PlanUnit, UnitEstimate};
use hades_core::jsfmt::to_fixed;

/// `SECTION_TITLE` (`src/lib/moirai.ts:8`).
pub const SECTION_TITLE: &str = "Estimativa";

/// `TABLE_HEADER` (`src/lib/moirai.ts:337`).
pub const TABLE_HEADER: [&str; 11] = [
    "Unidade",
    "Custo p25 (US$)",
    "Custo mediana (US$)",
    "Custo p75 (US$)",
    "Duração p25 (min)",
    "Duração mediana (min)",
    "Duração p75 (min)",
    "Gate mediana (min)",
    "Amostra",
    "Confiança",
    "Base",
];

const DASH: &str = "—";

static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\|?\s*:?-{2,}").expect("regex fixa"));
static UNITS_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^## Unidades\s*$").expect("regex fixa"));
static SECTION_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^## {SECTION_TITLE}\s*$")).expect("regex fixa"));

/// `cells` (`src/lib/moirai.ts:225`).
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_owned()).collect()
}

/// `isRule` (`src/lib/moirai.ts:226`).
fn is_rule(line: &str) -> bool {
    RULE.is_match(line.trim())
}

/// `stripTicks` (`src/lib/moirai.ts:227`).
fn strip_ticks(text: &str) -> String {
    text.replace('`', "").trim().to_owned()
}

/// `planUnits` (`src/lib/moirai.ts:230`): tabela sob `## Unidades` (colunas Unidade, Papel e Escopo ou Specs); vazio se
/// o plano não tem a tabela. Bloco de código é ignorado.
pub fn plan_units(plan_text: &str) -> Vec<PlanUnit> {
    let text = plan_text.replace('\r', "");
    let mut in_fence = false;
    let mut in_units = false;
    let mut header: Option<Vec<String>> = None;
    let mut units = Vec::new();
    for line in text.split('\n') {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        if line.starts_with("## ") {
            in_units = UNITS_TITLE.is_match(line);
            header = None;
            continue;
        }
        if !in_units || !line.trim().starts_with('|') {
            continue;
        }
        let Some(head) = header.as_ref() else {
            header = Some(cells(line).iter().map(|c| c.to_lowercase()).collect());
            continue;
        };
        if is_rule(line) {
            continue;
        }
        let row = cells(line);
        let col = |names: &[&str]| head.iter().position(|h| names.contains(&h.as_str()));
        let unit = col(&["unidade"]);
        let role = col(&["papel"]);
        let scope_at = col(&["escopo", "specs"]);
        let Some(unit) = unit.filter(|u| *u < row.len()) else {
            continue;
        };
        let scope = match scope_at {
            None => String::new(),
            Some(at) if at == head.len() - 1 => {
                row.get(at..).map(|r| r.join("|")).unwrap_or_default()
            }
            Some(at) => row.get(at).cloned().unwrap_or_default(),
        };
        let role = match role {
            None => "impl".to_owned(),
            Some(r) => {
                let value =
                    strip_ticks(row.get(r).map(String::as_str).unwrap_or("")).to_lowercase();
                if value.is_empty() {
                    "impl".to_owned()
                } else {
                    value
                }
            }
        };
        units.push(PlanUnit {
            name: strip_ticks(&row[unit]),
            role,
            scope,
        });
    }
    units
}

fn money(n: f64) -> String {
    to_fixed(n, 2)
}

fn minutes(n: f64) -> String {
    to_fixed(n, 0)
}

/// `estimateRow` (`src/lib/moirai.ts:348`).
pub fn estimate_row(e: &UnitEstimate) -> Vec<String> {
    let cell = |v: Option<f64>, f: fn(f64) -> String| v.map_or_else(|| DASH.to_owned(), f);
    vec![
        e.unit.clone(),
        cell(e.cost_usd.map(|c| c.p25), money),
        cell(e.cost_usd.map(|c| c.median), money),
        cell(e.cost_usd.map(|c| c.p75), money),
        cell(e.wall_min.map(|w| w.p25), minutes),
        cell(e.wall_min.map(|w| w.median), minutes),
        cell(e.wall_min.map(|w| w.p75), minutes),
        cell(e.gate_min.map(|g| g.median), minutes),
        e.sample.to_string(),
        e.confidence.as_str().to_owned(),
        e.basis.as_str().to_owned(),
    ]
}

/// `SectionInput` (`src/lib/moirai.ts:359`).
pub struct SectionInput<'a> {
    pub date: &'a str,
    pub estimates: &'a [UnitEstimate],
    pub history_size: usize,
    pub window_days: f64,
    pub params: Params,
}

/// Inteiro do JS impresso por interpolação (`${n}`).
fn int_text(n: f64) -> String {
    to_fixed(n, 0)
}

/// `renderSection` (`src/lib/moirai.ts:362`).
pub fn render_section(input: &SectionInput) -> String {
    let SectionInput {
        estimates, params, ..
    } = input;
    let window = if input.window_days > 0.0 {
        format!(" (últimos {} dias)", int_text(input.window_days))
    } else {
        String::new()
    };
    let mut out = vec![
        format!("## {SECTION_TITLE}"),
        "<!-- gerada por `hades moirai estimate --write`; fica dentro do hash da aprovação: reestimar pede nova aprovação -->".to_owned(),
        format!(
            "Gerada em {} · histórico: {} unidades encerradas{window} · amostra mínima {} · p25–p75 = faixa, o teto de custo da execução é o p75",
            input.date,
            input.history_size,
            int_text(params.min_sample)
        ),
        String::new(),
        format!("| {} |", TABLE_HEADER.join(" | ")),
        format!("|{}|", TABLE_HEADER.map(|_| "---").join("|")),
    ];
    out.extend(
        estimates
            .iter()
            .map(|e| format!("| {} |", estimate_row(e).join(" | "))),
    );
    let lows: Vec<&UnitEstimate> = estimates
        .iter()
        .filter(|e| {
            matches!(
                e.confidence,
                crate::model::Confidence::Baixa | crate::model::Confidence::Nenhuma
            ) || e.declared_lines.is_none()
        })
        .collect();
    if !lows.is_empty() {
        out.push(String::new());
        out.extend(lows.iter().map(|e| {
            format!(
                "- **Confiança {}** em `{}`: {}.",
                e.confidence.as_str(),
                e.unit,
                e.note
            )
        }));
    }
    format!("{}\n", out.join("\n"))
}

/// `writeSection` (`src/lib/moirai.ts:381`): troca a seção `## Estimativa` (ou a acrescenta no fim); o resto do plano
/// fica byte a byte igual.
pub fn write_section(plan_text: &str, section: &str) -> String {
    let lines: Vec<&str> = plan_text.split('\n').collect();
    let mut in_fence = false;
    let mut start: Option<usize> = None;
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence || !line.starts_with("## ") {
            continue;
        }
        if start.is_some() {
            end = i;
            break;
        }
        if SECTION_LINE.is_match(line) {
            start = Some(i);
        }
    }
    let Some(start) = start else {
        let trimmed = plan_text.trim_end_matches('\n');
        return format!("{trimmed}\n\n{section}");
    };
    let tail = lines[end..].join("\n");
    let head = lines[..start].join("\n");
    if tail.is_empty() {
        format!("{head}\n{section}")
    } else {
        format!("{head}\n{section}\n{tail}")
    }
}

/// `hasSection` (`src/lib/moirai.ts:401`).
pub fn has_section(plan_text: &str) -> bool {
    let mut in_fence = false;
    for line in plan_text.replace('\r', "").split('\n') {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && SECTION_LINE.is_match(line) {
            return true;
        }
    }
    false
}

/// Uma linha da tabela lida de volta (`ReturnType<typeof parseSection>`, `src/lib/moirai.ts:412`); `None` onde há "—".
#[derive(Clone, Debug, PartialEq)]
pub struct EstimateRow {
    pub unit: String,
    pub cost_p25: Option<f64>,
    pub cost_median: Option<f64>,
    pub cost_p75: Option<f64>,
    pub wall_p25: Option<f64>,
    pub wall_median: Option<f64>,
    pub wall_p75: Option<f64>,
    pub gate_median: Option<f64>,
    /// `Number(row[8] ?? 0)`: pode ser `NaN`.
    pub sample: f64,
    pub confidence: String,
    pub basis: String,
}

/// `parseSection` (`src/lib/moirai.ts:412`): uma linha por unidade da seção `## Estimativa`.
pub fn parse_section(plan_text: &str) -> Vec<EstimateRow> {
    let text = plan_text.replace('\r', "");
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(at) = lines.iter().position(|l| SECTION_LINE.is_match(l)) else {
        return Vec::new();
    };
    let num = |cell: Option<&String>| -> Option<f64> {
        let cell = cell?;
        if cell == DASH {
            return None;
        }
        js_number(cell)
    };
    let mut rows = Vec::new();
    for line in &lines[at + 1..] {
        if line.starts_with("## ") {
            break;
        }
        if !line.trim().starts_with('|') || is_rule(line) {
            continue;
        }
        let row = cells(line);
        if row[0] == TABLE_HEADER[0] {
            continue;
        }
        rows.push(EstimateRow {
            unit: row[0].clone(),
            cost_p25: num(row.get(1)),
            cost_median: num(row.get(2)),
            cost_p75: num(row.get(3)),
            wall_p25: num(row.get(4)),
            wall_median: num(row.get(5)),
            wall_p75: num(row.get(6)),
            gate_median: num(row.get(7)),
            sample: row.get(8).map_or(0.0, |c| js_number(c).unwrap_or(f64::NAN)),
            confidence: row.get(9).cloned().unwrap_or_default(),
            basis: row.get(10).cloned().unwrap_or_default(),
        });
    }
    rows
}

/// `estimateOfUnit` (`src/lib/moirai.ts:498`): a linha da unidade na seção do plano; `None` se o plano não a tem.
pub fn estimate_of_unit(plan_text: &str, unit: &str) -> Option<EstimateRow> {
    parse_section(plan_text)
        .into_iter()
        .find(|row| row.unit == unit)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::estimate::estimate_plan;
    use crate::estimate::fixtures::many;
    use crate::model::{Band, Mode};
    use hades_core::registry::plan_sha;

    const PARAMS: Params = Params {
        min_sample: 5.0,
        high_factor: 3.0,
        small_lines: 150.0,
        large_lines: 600.0,
    };

    const PLAN: &str = "# Plano\n\n## Estado atual\n- tudo certo\n\n## Unidades\n| Unidade | Papel | Base | Escopo |\n|---|---|---|---|\n| `alfa` | impl | main | mexe em ~300 linhas com `a | b` |\n| beta | spec | alfa | 5 arquivos |\n\n## Riscos\nnenhum\n";

    fn pu(name: &str, role: &str, scope: &str) -> PlanUnit {
        PlanUnit {
            name: name.into(),
            role: role.into(),
            scope: scope.into(),
        }
    }

    #[test]
    fn dado_a_tabela_entao_le_unidade_papel_e_escopo_com_a_barra_dentro_do_escopo() {
        assert_eq!(
            plan_units(PLAN),
            vec![
                pu("alfa", "impl", "mexe em ~300 linhas com `a|b`"),
                pu("beta", "spec", "5 arquivos")
            ]
        );
    }

    #[test]
    fn dado_plano_sem_tabela_de_unidades_entao_nenhuma_unidade() {
        assert_eq!(plan_units("# Plano\n\n## Objetivo\ntexto\n"), vec![]);
    }

    #[test]
    fn dado_a_tabela_do_modelo_com_specs_e_card_entao_a_unidade_vem_pelo_cabecalho() {
        let text = "## Unidades\n| Unidade | Card | Papel | Modelo/esforço | Base | Depende de | Specs |\n|---|---|---|---|---|---|---|\n| x | novo | impl | — | main | — | A, B |\n";
        assert_eq!(plan_units(text), vec![pu("x", "impl", "A, B")]);
    }

    #[test]
    fn dado_bloco_de_codigo_com_tabela_falsa_entao_e_ignorado() {
        assert_eq!(
            plan_units("```\n## Unidades\n| Unidade | Papel |\n|---|---|\n| x | impl |\n```\n"),
            vec![]
        );
    }

    #[test]
    fn dado_tabela_sem_coluna_de_papel_e_celula_vazia_entao_o_papel_e_impl() {
        let text = "## Unidades\n| Unidade | Escopo |\n|---|---|\n| x | ~10 linhas |\n";
        assert_eq!(plan_units(text), vec![pu("x", "impl", "~10 linhas")]);
        let text = "## Unidades\n| Unidade | Papel |\n|---|---|\n| y |  |\n";
        assert_eq!(plan_units(text), vec![pu("y", "impl", "")]);
    }

    fn section() -> String {
        let history = many(6, |i, u| {
            u.cost_usd = Some(i as f64 + 1.0);
            u.wall_s = Some(600.0);
            u.lines = Some(300.0);
            u.band = Some(Band::Media);
        });
        let estimates = estimate_plan(
            &plan_units(PLAN),
            Mode::Skill,
            &history,
            &PARAMS,
            &HashMap::new(),
        );
        render_section(&SectionInput {
            date: "2026-10-28",
            estimates: &estimates,
            history_size: 6,
            window_days: 90.0,
            params: PARAMS,
        })
    }

    #[test]
    fn dado_a_estimativa_entao_a_secao_e_gravada_no_fim_e_o_resto_do_plano_fica_igual() {
        let written = write_section(PLAN, &section());
        assert!(written.starts_with(PLAN.trim_end()));
        assert!(!has_section(PLAN));
        assert!(has_section(&written));
    }

    #[test]
    fn dado_plano_que_ja_tem_a_secao_no_meio_entao_ela_e_trocada_e_a_seguinte_fica() {
        let sec = section();
        let with = PLAN.replace("## Riscos", &format!("{sec}\n## Riscos"));
        let first = write_section(&with, &sec.replace("2026-10-28", "2026-11-01"));
        assert!(first.contains("2026-11-01"));
        assert!(!first.contains("2026-10-28"));
        assert!(first.contains("## Riscos\nnenhum"));
        assert_eq!(first.lines().filter(|l| *l == "## Estimativa").count(), 1);
    }

    #[test]
    fn dado_gravar_duas_vezes_igual_entao_e_idempotente() {
        let sec = section();
        let once = write_section(PLAN, &sec);
        assert_eq!(write_section(&once, &sec), once);
    }

    #[test]
    fn dado_a_secao_gravada_entao_ela_entra_no_hash_da_aprovacao_e_o_estado_atual_segue_fora() {
        let written = write_section(PLAN, &section());
        assert_ne!(plan_sha(&written), plan_sha(PLAN));
        assert_eq!(
            plan_sha(&written.replace("tudo certo", "outra coisa")),
            plan_sha(&written)
        );
    }

    #[test]
    fn dado_confianca_baixa_entao_o_aviso_esta_no_texto_da_secao() {
        let low = estimate_plan(
            &plan_units(PLAN),
            Mode::Skill,
            &many(2, |_, _| {}),
            &PARAMS,
            &HashMap::new(),
        );
        let text = render_section(&SectionInput {
            date: "d",
            estimates: &low,
            history_size: 2,
            window_days: 0.0,
            params: PARAMS,
        });
        assert!(text.contains("**Confiança baixa** em `alfa`"));
        assert!(!text.contains("últimos"));
    }

    #[test]
    fn dado_a_secao_gravada_entao_parse_section_devolve_os_numeros_de_volta() {
        let rows = parse_section(&write_section(PLAN, &section()));
        let alfa = &rows[0];
        assert_eq!(
            (
                alfa.unit.as_str(),
                alfa.cost_median,
                alfa.sample,
                alfa.basis.as_str()
            ),
            ("alfa", Some(3.5), 6.0, "parecidas")
        );
        assert!(alfa.cost_p75.unwrap() > alfa.cost_p25.unwrap());
        assert_eq!(
            estimate_of_unit(&write_section(PLAN, &section()), "beta").map(|r| r.unit),
            Some("beta".to_owned())
        );
        assert_eq!(estimate_of_unit(PLAN, "alfa"), None);
    }

    #[test]
    fn dado_plano_sem_quebra_final_entao_a_secao_entra_depois_de_uma_linha_em_branco() {
        assert_eq!(
            write_section("# P", "## Estimativa\nx\n"),
            "# P\n\n## Estimativa\nx\n"
        );
    }

    #[test]
    fn dado_dinheiro_e_minutos_entao_arredonda_como_o_to_fixed_do_js() {
        let mut e = estimate_plan(
            &[pu("a", "impl", "")],
            Mode::Skill,
            &many(5, |_, u| u.cost_usd = Some(0.125)),
            &PARAMS,
            &HashMap::new(),
        );
        let row = estimate_row(&e.remove(0));
        assert_eq!(&row[1..4], ["0.13", "0.13", "0.13"]);
    }
}
