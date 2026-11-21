use std::fs;
use std::process::Command;

use serde_json::json;

use super::*;
use crate::model::Band;

fn params() -> Params {
    Params {
        min_sample: 5.0,
        high_factor: 3.0,
        small_lines: 150.0,
        large_lines: 600.0,
    }
}

fn record(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn unit(name: &str, created: &str, done: &str) -> HistoryUnit {
    history_unit(
        &record(json!({"name": name, "state": "done", "createdAt": created, "doneAt": done})),
        "r",
        &Measured::default(),
        &params(),
    )
}

#[test]
fn dado_registro_encerrado_sem_transcript_nem_worktree_entao_e_fixture() {
    let r = record(json!({"name": "x", "state": "done"}));
    assert!(!is_real_unit(&r, false, false));
}

#[test]
fn dado_transcript_ou_worktree_entao_entra_e_em_andamento_nunca() {
    assert!(is_real_unit(
        &record(json!({"name": "x", "state": "archived"})),
        true,
        false
    ));
    assert!(is_real_unit(
        &record(json!({"name": "x", "state": "done"})),
        false,
        true
    ));
    assert!(!is_real_unit(
        &record(json!({"name": "x", "state": "working"})),
        true,
        true
    ));
    assert!(!is_real_unit(
        &record(json!({"name": "", "state": "done"})),
        true,
        true
    ));
}

#[test]
fn dado_registro_de_skill_entao_relogio_vai_de_created_a_done() {
    let r = record(json!({
        "name": "a", "mode": "skill", "role": "impl", "state": "done",
        "createdAt": "2026-10-01T00:00:00Z", "doneAt": "2026-10-01T00:30:00Z", "clears": 2
    }));
    let measured = Measured {
        lines: Some(200.0),
        ..Default::default()
    };
    let h = history_unit(&r, "_skill", &measured, &params());
    assert_eq!(
        (h.mode, h.wall_s, h.clears, h.band, h.resumes),
        (Mode::Skill, Some(1800.0), 2.0, Some(Band::Media), None)
    );
}

#[test]
fn dado_adotada_e_arquivada_entao_usa_adopted_e_archived_e_papel_padrao_impl() {
    let r = record(json!({
        "name": "a", "state": "archived", "adoptedAt": "2026-10-01T00:00:00Z",
        "archivedAt": "2026-10-01T01:00:00Z", "clears": -1, "resumes": 3
    }));
    let h = history_unit(
        &r,
        "r",
        &Measured {
            lines: Some(0.0),
            ..Default::default()
        },
        &params(),
    );
    assert_eq!(h.created_at, "2026-10-01T00:00:00Z");
    assert_eq!(h.done_at, "2026-10-01T01:00:00Z");
    assert_eq!(
        (h.role.as_str(), h.clears, h.resumes, h.band),
        ("impl", 0.0, Some(3.0), None)
    );
}

#[test]
fn dado_fim_antes_do_inicio_ou_data_ruim_entao_sem_relogio() {
    assert_eq!(
        seconds("2026-10-02T00:00:00Z", "2026-10-01T00:00:00Z"),
        None
    );
    assert_eq!(seconds("", "2026-10-01T00:00:00Z"), None);
}

#[test]
fn dado_execucoes_em_dois_historicos_entao_soma_tempo_pega_pico_e_ignora_repetida() {
    let line = |label: &str, dur: u32, peak: u32| {
        format!(
            "2026-10-01T00:00:00Z\tdone\ttest\t{label}\t4\t/wt\trc=0\tdur={dur}s\tpeak={peak}MB\tcmd=x"
        )
    };
    let a = format!("{}\n{}\n", line("u", 60, 100), line("outro", 999, 9999));
    let b = format!("{}\n{}\n", line("u", 120, 300), line("u", 60, 100));
    assert_eq!(gate_of_label(&[a, b], "u"), (180.0, Some(300.0)));
}

#[test]
fn dado_label_que_nunca_passou_pelo_gate_entao_zero_e_sem_pico() {
    let journal = "x\tdone\tt\tz\t1\t/w\trc=0\tdur=5s\n".to_owned();
    assert_eq!(gate_of_label(&[journal], "u"), (0.0, None));
}

#[test]
fn dado_a_mesma_unidade_em_duas_raizes_entao_fica_a_de_done_mais_novo_ordenada() {
    let mut velha = unit("a", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z");
    velha.root = "x".into();
    let mut nova = unit("a", "2026-10-01T00:00:00Z", "2026-10-03T00:00:00Z");
    nova.root = "y".into();
    let antes = unit("b", "2026-10-01T00:00:00Z", "2026-10-02T12:00:00Z");
    let out = dedupe(vec![velha, antes, nova]);
    let seen: Vec<(&str, &str)> = out
        .iter()
        .map(|u| (u.name.as_str(), u.root.as_str()))
        .collect();
    assert_eq!(seen, [("b", "r"), ("a", "y")]);
}

#[test]
fn dado_janela_entao_so_o_encerrado_dentro_dela_e_zero_vale_tudo() {
    let now = 1_795_176_000_000;
    let dentro = unit("d", "2026-11-01T00:00:00Z", "2026-11-19T10:00:00Z");
    let fora = unit("f", "2026-01-01T00:00:00Z", "2026-02-01T10:00:00Z");
    let kept = in_window(vec![dentro.clone(), fora.clone()], now, 90.0);
    assert_eq!(kept, vec![dentro.clone()]);
    assert_eq!(in_window(vec![dentro, fora], now, 0.0).len(), 2);
}

#[test]
fn dado_unidades_entao_o_texto_alinha_colunas_com_traco_para_o_que_falta() {
    let mut u = unit(
        "uma-unidade-com-nome-bem-comprido-demais",
        "2026-10-01T00:00:00Z",
        "2026-10-01T00:30:00Z",
    );
    u.cost_usd = Some(1.005);
    u.tokens = Some(2_500_000.0);
    u.gate_s = 180.0;
    u.gate_peak_mb = Some(1536.0);
    u.files = Some(3.0);
    u.lines = Some(200.0);
    u.band = Some(Band::Media);
    let lines = history_lines(&[u, unit("b", "", "")]);
    assert_eq!(
        lines[0],
        "unidade                   papel       modo       custo   tokens  relógio   gate    pico clr  arq  linhas  tam"
    );
    assert_eq!(
        lines[1],
        "uma-unidade-com-nome-bem- impl        projeto    $1.00     2.5M      30m     3m    1.5G   0    3     200  media"
    );
    assert_eq!(
        lines[2],
        "b                         impl        projeto        —        —        —     0m       —   0    —       —  —"
    );
}

#[test]
fn dado_nenhuma_unidade_entao_o_json_e_so_a_lista_vazia() {
    assert_eq!(history_json(&[]), "{\n \"v\": 1,\n \"units\": []\n}");
}

fn git(dir: &Path, date: &str, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn dado_registros_worktree_e_git_entao_coleta_mede_e_filtra() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let env = Env::empty()
        .with("HOME", h.to_str().unwrap())
        .with("HADES_WORK_ROOT", h.join("work").to_str().unwrap())
        .with("HADES_SKILL_DIR", h.join("nada").to_str().unwrap())
        .with("HADES_NOW", "2026-11-20T12:00:00Z");
    let wt = h.join("wt").join("u-um");
    fs::create_dir_all(&wt).unwrap();
    git(&wt, "2026-11-01T00:00:00Z", &["init", "-q", "-b", "feat"]);
    fs::write(wt.join("a.txt"), "1\n2\n3\n").unwrap();
    git(&wt, "2026-11-01T00:10:00Z", &["add", "."]);
    git(&wt, "2026-11-01T00:10:00Z", &["commit", "-q", "-m", "um"]);
    fs::write(wt.join("b.txt"), "x\n").unwrap();
    git(&wt, "2026-11-01T00:20:00Z", &["add", "."]);
    git(&wt, "2026-11-01T00:20:00Z", &["commit", "-q", "-m", "dois"]);
    git(
        &wt,
        "2026-11-05T00:20:00Z",
        &["commit", "-q", "--allow-empty", "-m", "fora"],
    );

    let agents = h.join("work/acme__demo/orchestration/agents");
    fs::create_dir_all(&agents).unwrap();
    let reg = |name: &str, extra: Value| {
        let mut r = json!({
            "name": name, "state": "done", "role": "impl", "branch": "feat",
            "worktree": wt.to_str().unwrap(),
            "createdAt": "2026-11-01T00:00:00Z", "doneAt": "2026-11-01T01:00:00Z",
        });
        r.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        fs::write(agents.join(format!("{name}.json")), r.to_string()).unwrap();
    };
    reg("u-um", json!({}));
    reg("u-fixture", json!({"worktree": "", "branch": ""}));
    reg(
        "u-velha",
        json!({"doneAt": "2026-01-01T01:00:00Z", "createdAt": "2026-01-01T00:00:00Z"}),
    );
    fs::write(agents.join("quebrado.json"), "{").unwrap();
    let journal =
        "2026-11-01T00:30:00Z\tdone\ttest\tu-um\t4\t/wt\trc=0\tdur=90s\tpeak=2048MB\tcmd=x\n";
    fs::create_dir_all(h.join("work/_maquina/locks")).unwrap();
    fs::write(h.join("work/_maquina/locks/gate-journal.tsv"), journal).unwrap();

    let units = collect_history(&env, h, &params()).unwrap();
    let names: Vec<&str> = units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["u-um"]);
    let u = &units[0];
    assert_eq!(
        (u.files, u.lines, u.band),
        (Some(2.0), Some(4.0), Some(Band::Pequena))
    );
    assert_eq!(
        (u.gate_s, u.gate_peak_mb, u.wall_s),
        (90.0, Some(2048.0), Some(3600.0))
    );
    assert_eq!((u.cost_usd, u.tokens), (None, None));

    let all = collect_history(
        &env.clone().with("HADES_MOIRAI_WINDOW_DAYS", "0"),
        h,
        &params(),
    )
    .unwrap();
    assert_eq!(all.len(), 2);
    let bad = collect_history(&env.with("HADES_MOIRAI_WINDOW_DAYS", "-1"), h, &params());
    assert_eq!(bad.unwrap_err().code, 2);
}
