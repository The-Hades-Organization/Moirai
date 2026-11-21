//! Dicas do `--deep` (`src/commands/moirai.ts:213-259`): o subagente estima as linhas de cada unidade.

use std::collections::HashMap;

use hades_core::output::warn;
use regex::Regex;

use crate::model::PlanUnit;

/// `DEEP_MAX_LINES` (`src/commands/moirai.ts:34`).
pub const DEEP_MAX_LINES: u64 = 100_000;

/// `parseDeepAnswer` (`src/commands/moirai.ts:243`): a resposta só vale com `{"lines": N}`, 0 < N ≤ 100000.
pub fn parse_deep_answer(answer: &str) -> Option<u64> {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r#"\{[^{}]*"lines"\s*:\s*(\d+)[^{}]*\}"#).unwrap());
    let lines: u64 = RE.captures(answer)?[1].parse().ok()?;
    (lines > 0 && lines <= DEEP_MAX_LINES).then_some(lines)
}

/// Prompt de uma unidade (`src/commands/moirai.ts:252`).
pub fn deep_prompt(unit: &PlanUnit) -> String {
    format!(
        "Estime o tamanho de implementação (linhas adicionadas + removidas no diff) desta unidade de um plano. Leia o código que o escopo cita, se precisar. Responda só com um JSON {{\"lines\": <inteiro>}}.\n\nUnidade: {}\nPapel: {}\nEscopo:\n{}",
        unit.name, unit.role, unit.scope
    )
}

/// `deepHints` (`src/commands/moirai.ts:249`). O `runner` devolve o stdout do estimador, ou `None` se ele falhou.
pub fn deep_hints<E>(
    units: &[PlanUnit],
    runner: &mut dyn FnMut(&str) -> Result<Option<String>, E>,
) -> Result<HashMap<String, f64>, E> {
    let mut hints = HashMap::new();
    for unit in units {
        let answer = runner(&deep_prompt(unit))?.unwrap_or_default();
        match parse_deep_answer(&answer) {
            Some(lines) => {
                hints.insert(unit.name.clone(), lines as f64);
            }
            None => warn(&format!(
                "moirai --deep: sem resposta utilizável para {}; fica o tamanho do escopo",
                unit.name
            )),
        }
    }
    Ok(hints)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str) -> PlanUnit {
        PlanUnit {
            name: name.into(),
            role: "impl".into(),
            scope: "4 arquivos".into(),
        }
    }

    #[test]
    fn aceita_o_json_no_meio_de_texto() {
        assert_eq!(parse_deep_answer("ok {\"lines\": 420} fim"), Some(420));
        assert_eq!(parse_deep_answer("{\"x\":1,\"lines\":7,\"y\":2}"), Some(7));
    }

    #[test]
    fn recusa_zero_acima_do_teto_e_lixo() {
        assert_eq!(parse_deep_answer("{\"lines\": 0}"), None);
        assert_eq!(parse_deep_answer("{\"lines\": 100001}"), None);
        assert_eq!(parse_deep_answer("{\"lines\": 100000}"), Some(100_000));
        assert_eq!(parse_deep_answer("{\"lines\": -3}"), None);
        assert_eq!(parse_deep_answer("{\"lines\": \"9\"}"), None);
        assert_eq!(parse_deep_answer("sem json"), None);
        assert_eq!(parse_deep_answer("{\"lines\": 99999999999999999999}"), None);
    }

    #[test]
    fn prompt_traz_unidade_papel_e_escopo() {
        let p = deep_prompt(&unit("u1"));
        assert!(p.ends_with("Unidade: u1\nPapel: impl\nEscopo:\n4 arquivos"));
        assert!(p.contains("{\"lines\": <inteiro>}"));
    }

    #[test]
    fn erro_do_runner_interrompe_sem_chamar_as_outras() {
        let units = [unit("a"), unit("b")];
        let mut calls = 0;
        let r = deep_hints(&units, &mut |_| {
            calls += 1;
            Err("quebrou")
        });
        assert_eq!(r, Err("quebrou"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn so_entram_as_unidades_com_resposta_valida() {
        let units = [unit("a"), unit("b"), unit("c")];
        let mut answers = vec![
            Some("{\"lines\": 50}".to_owned()),
            None,
            Some("lixo".into()),
        ]
        .into_iter();
        let hints = deep_hints::<()>(&units, &mut |_| Ok(answers.next().unwrap())).unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints["a"], 50.0);
    }
}
