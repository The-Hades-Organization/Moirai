//! Casos de `loop/test/ts/moirai-real.test.ts`: "limiar do p75" e "rotas do painel".

use std::fs;

use serde_json::json;

use super::*;
use crate::plan::TABLE_HEADER;
use crate::real::encode_cwd;

fn plan(unit: &str, p75: &str) -> String {
    let rule = "---|".repeat(TABLE_HEADER.len());
    format!(
        "# plano\n\n## Unidades\n\n| Unidade | Papel | Escopo |\n|---|---|---|\n| {unit} | impl | x |\n\n## Estimativa\n\n| {} |\n|{rule}\n| {unit} | 0.50 | 0.80 | {p75} | 1 | 2 | 3 | — | 9 | media | parecidas |\n",
        TABLE_HEADER.join(" | ")
    )
}

fn line(id: &str, output_tokens: u64) -> String {
    format!(
        "{}\n",
        json!({"type": "assistant", "requestId": id, "timestamp": "2026-10-01T10:00:00Z",
            "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": output_tokens}}})
    )
}

#[test]
fn abaixo_do_p75_nada_passa_de_limiar() {
    let b = budget_of(0.99, Some(1.0), 100.0, 150.0);
    assert!(!b.warn && !b.ask);
    assert_eq!(b.level, BudgetLevel::Ok);
}

#[test]
fn em_100_por_cento_avisa_e_ainda_nao_pergunta() {
    let b = budget_of(1.0, Some(1.0), 100.0, 150.0);
    assert_eq!(b.pct, Some(100.0));
    assert!(b.warn && !b.ask);
    assert_eq!(b.level, BudgetLevel::Warn);
}

#[test]
fn em_150_por_cento_pergunta() {
    let b = budget_of(1.5, Some(1.0), 100.0, 150.0);
    assert!(b.warn && b.ask);
    assert_eq!(b.level, BudgetLevel::Ask);
}

#[test]
fn limiares_da_configuracao_valem_no_lugar_dos_padroes() {
    let b = budget_of(0.6, Some(1.0), 50.0, 80.0);
    assert!(b.warn && !b.ask);
}

#[test]
fn p75_ausente_ou_zero_nao_da_percentual_nem_aviso() {
    for p75 in [None, Some(0.0)] {
        assert_eq!(
            budget_of(99.0, p75, 100.0, 150.0),
            Budget {
                pct: None,
                warn: false,
                ask: false,
                level: BudgetLevel::Ok
            }
        );
    }
}

#[test]
fn linha_da_unidade_na_secao_estimativa() {
    assert_eq!(
        estimate_of_unit(&plan("u1", "1.00"), "u1")
            .unwrap()
            .cost_p75,
        Some(1.0)
    );
    assert!(estimate_of_unit(&plan("u1", "1.00"), "outra").is_none());
    assert!(estimate_of_unit("# plano sem seção\n", "u1").is_none());
}

#[test]
fn proxima_pergunta_e_a_maior_mais_um_e_a_do_teto_leva_as_tres_saidas() {
    assert_eq!(next_question_number(""), 1);
    assert_eq!(
        next_question_number("## Q1 — a\n## Q4 — b (respondida)\n"),
        5
    );
    let q = budget_question(&BudgetQuestionInput {
        number: 2,
        unit: "u1".into(),
        phase: "impl".into(),
        cost_usd: 2.0,
        p75: 1.0,
        pct: 200.0,
        ask_pct: 150.0,
    });
    assert!(q.starts_with("## Q2 — custo passou de 150% do estimado"));
    let (a, b, c) = (
        q.find("Seguir").unwrap(),
        q.find("Reestimar").unwrap(),
        q.find("Estacionar").unwrap(),
    );
    assert!(a < b && b < c);
    assert!(q.contains("US$ 2.00, 200% do p75 da estimativa do plano (US$ 1.00)"));
}

#[test]
fn pergunta_do_limiar_ja_gravada_vale_por_fase() {
    let q = budget_question(&BudgetQuestionInput {
        number: 1,
        unit: "u1".into(),
        phase: "impl".into(),
        cost_usd: 2.0,
        p75: 1.0,
        pct: 200.0,
        ask_pct: 150.0,
    });
    let text = format!("## Q0 — outra\n\n{q}## Q2 — outra (fase impl)\n");
    assert!(has_budget_question(&text, 150.0, "impl"));
    assert!(!has_budget_question(&text, 150.0, "review"));
    assert!(!has_budget_question(&text, 200.0, "impl"));
    assert!(!has_budget_question("", 150.0, ""));
}

struct Painel {
    _dir: tempfile::TempDir,
    routes: MoiraiRoutes,
}

fn painel() -> Painel {
    let dir = tempfile::tempdir().unwrap();
    let t = dir.path();
    let orch = t.join("panelwork/ex__proj/orchestration");
    let worktree = format!("{}/wt-panel", t.display());
    let projects = t.join("projects-panel");
    fs::create_dir_all(orch.join("agents")).unwrap();
    fs::create_dir_all(projects.join(encode_cwd(&worktree))).unwrap();
    fs::write(
        projects.join(encode_cwd(&worktree)).join("s.jsonl"),
        line("a", 100_000),
    )
    .unwrap();
    let plan_path = t.join("plan-panel.md");
    fs::write(&plan_path, plan("u-um", "1.00")).unwrap();
    fs::write(
        orch.join("plan.json"),
        json!({"path": plan_path.display().to_string(), "sha": "x"}).to_string(),
    )
    .unwrap();
    fs::write(
        orch.join("agents/u-um.json"),
        json!({"name": "u-um", "role": "impl", "worktree": worktree, "state": "working",
            "createdAt": "2026-10-01T10:00:00Z"})
        .to_string(),
    )
    .unwrap();
    let registry = Registry {
        slug: "ex__proj".into(),
        orch,
    };
    let routes = MoiraiRoutes::new(MoiraiDeps {
        registries: Box::new(move || vec![registry.clone()]),
        config: Box::new(HashMap::new),
        projects_dir: projects,
        now_ms: Box::new(|| parse_ts_ms("2026-10-01T11:00:00Z")),
    });
    Painel { _dir: dir, routes }
}

#[test]
fn plano_traz_estimado_e_real_com_nivel_ask() {
    let mut p = painel();
    let out = p.routes.plan();
    assert_eq!(out["v"], 1);
    assert_eq!(out["warnPct"], 100);
    assert_eq!(out["askPct"], 150);
    let plan = &out["plans"][0];
    assert_eq!(plan["registry"], "ex__proj");
    assert_eq!(plan["approved"], false);
    let unit = &plan["units"][0];
    assert_eq!(unit["unit"], "u-um");
    assert_eq!(unit["level"], "ask");
    assert!((unit["pct"].as_f64().unwrap() - 200.0).abs() < 1e-3);
    assert!((unit["real"]["wallMin"].as_f64().unwrap() - 60.0).abs() < 1e-3);
    assert_eq!(unit["real"]["state"], "working");
    assert_eq!(unit["real"]["phase"], "impl");
    assert_eq!(unit["estimate"]["costP75"], 1);
    assert_eq!(unit["estimate"]["gateMedian"], serde_json::Value::Null);
}

#[test]
fn unidade_registrada_sai_com_o_mesmo_estimado_x_real() {
    let mut p = painel();
    let out = p.routes.unit("u-um", Some("ex__proj")).unwrap();
    assert_eq!(out["unit"], "u-um");
    assert_eq!(out["level"], "ask");
    assert_eq!(out["registry"], "ex__proj");
    let chaves: Vec<&String> = out.as_object().unwrap().keys().collect();
    assert_eq!(
        chaves,
        [
            "v", "warnPct", "askPct", "registry", "unit", "estimate", "real", "pct", "level"
        ]
    );
}

#[test]
fn nome_invalido_da_400_com_codigo_estavel() {
    let mut p = painel();
    let longo = "x".repeat(101);
    for name in ["../x", "a/b", ".oculto", "", longo.as_str(), "a b"] {
        let err = p.routes.unit(name, None).unwrap_err();
        assert_eq!((err.status, err.code), (400, ERR_INVALID_UNIT), "{name}");
    }
}

#[test]
fn nome_bem_formado_fora_do_registro_da_404() {
    let mut p = painel();
    let err = p.routes.unit("nao-existe", None).unwrap_err();
    assert_eq!((err.status, err.code), (404, ERR_UNKNOWN_UNIT));
}

#[test]
fn registro_de_outro_projeto_na_query_nao_acha_a_unidade() {
    let mut p = painel();
    assert_eq!(
        p.routes
            .unit("u-um", Some("outro__proj"))
            .unwrap_err()
            .status,
        404
    );
}

#[test]
fn limiares_da_configuracao_so_valem_inteiro_positivo() {
    let mut p = painel();
    p.routes.deps.config = Box::new(|| {
        HashMap::from([
            ("HADES_MOIRAI_WARN_PCT".to_string(), "50".to_string()),
            ("HADES_MOIRAI_ASK_PCT".to_string(), "1.5".to_string()),
        ])
    });
    let out = p.routes.plan();
    assert_eq!(out["warnPct"], 50);
    assert_eq!(out["askPct"], 150);
}

#[test]
fn unidade_sem_linha_na_estimativa_sai_sem_nivel() {
    let mut p = painel();
    let dir = p.routes.deps.projects_dir.parent().unwrap().to_path_buf();
    let plan_path = dir.join("plan-panel.md");
    fs::write(&plan_path, "# plano sem seção\n").unwrap();
    let out = p.routes.unit("u-um", None).unwrap();
    assert_eq!(out["estimate"], serde_json::Value::Null);
    assert_eq!(out["level"], serde_json::Value::Null);
    assert_eq!(out["pct"], serde_json::Value::Null);
}
