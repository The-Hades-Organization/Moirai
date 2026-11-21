use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

fn assistant(ts: &str, id: &str, input: u64, read: u64, create: u64, side: bool) -> String {
    json!({
        "type": "assistant", "timestamp": ts, "sessionId": "S", "requestId": id, "cwd": "/wt/a",
        "isSidechain": side,
        "message": {"id": id, "model": "claude-sonnet-5-5", "usage": {
            "input_tokens": input, "output_tokens": 5,
            "cache_read_input_tokens": read, "cache_creation_input_tokens": create}}
    })
    .to_string()
}

fn write(dir: &tempfile::TempDir, name: &str, lines: &[String]) -> PathBuf {
    let path = dir.path().join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, lines.join("\n")).unwrap();
    path
}

fn rules() -> GuardRules {
    GuardRules::embedded()
}

fn bash_turn(ts: &str, id: &str, command: &str, background: bool) -> String {
    json!({
        "type": "assistant", "timestamp": ts, "sessionId": "S", "requestId": id,
        "message": {"id": id, "model": "claude-opus-5-5",
            "usage": {"input_tokens": 1, "output_tokens": 1000},
            "content": [{"type": "tool_use", "name": "Bash",
                "input": {"command": command, "run_in_background": background}}]}
    })
    .to_string()
}

#[test]
fn contexto_fixo_e_input_mais_leitura_mais_escrita_da_primeira_resposta() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "s.jsonl",
        &[
            assistant("2026-11-01T10:00:00Z", "a", 3, 1000, 40000, false),
            assistant("2026-11-01T10:01:00Z", "b", 3, 41000, 500, false),
            assistant("2026-11-02T10:01:00Z", "c", 3, 41500, 500, false),
        ],
    );
    let rows = parse_file(&path, &rules());
    let opens: Vec<Option<Open>> = rows.iter().map(|r| r.open.clone()).collect();
    assert_eq!(
        opens,
        [
            Some(Open {
                ctx: 41003.0,
                model: "sonnet-5-5".into()
            }),
            None
        ]
    );
    assert_eq!(rows[0].turns, 2);
    assert_eq!(rows[1].day, "2026-11-02");
}

#[test]
fn resposta_de_subagente_nao_conta_como_abertura() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "s.jsonl",
        &[assistant("2026-11-01T10:00:00Z", "a", 1, 2, 3, true)],
    );
    let rows = parse_file(&path, &rules());
    assert!(rows[0].sidechain);
    assert_eq!(rows[0].open, None);
}

#[test]
fn caminho_de_subagents_marca_sidechain() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "subagents/x.jsonl",
        &[assistant("2026-11-01T10:00:00Z", "a", 1, 2, 3, false)],
    );
    assert!(parse_file(&path, &rules())[0].sidechain);
}

#[test]
fn sessao_sem_resposta_nao_tem_linha() {
    let dir = tempfile::tempdir().unwrap();
    let user =
        json!({"type": "user", "timestamp": "2026-11-01T10:00:00Z", "message": {"content": "oi"}});
    let path = write(&dir, "s.jsonl", &[user.to_string()]);
    assert!(parse_file(&path, &rules()).is_empty());
    assert!(parse_file(&dir.path().join("nada.jsonl"), &rules()).is_empty());
}

#[test]
fn requestid_repetido_conta_uma_vez_e_a_sessao_cai_no_caminho_sem_sessionid() {
    let dir = tempfile::tempdir().unwrap();
    let line = json!({"type": "assistant", "requestId": "r1", "timestamp": "2026-11-01T01:00:00Z", "cwd": "/x",
        "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1, "output_tokens": 1000}}})
    .to_string();
    let path = write(&dir, "s.jsonl", &[line.clone(), line]);
    let rows = parse_file(&path, &rules());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].turns, 1);
    assert_eq!(rows[0].session, path.to_string_lossy());
    assert!((rows[0].cost - (4.0 + 20_000.0) / 1e6).abs() < 1e-12);
    assert_eq!(rows[0].efforts.get("?"), Some(&1));
}

#[test]
fn modelo_desconhecido_sem_horario_ou_sem_usage_nao_custa() {
    let dir = tempfile::tempdir().unwrap();
    let lines = [
        json!({"type": "assistant", "requestId": "a", "timestamp": "t", "message": {"model": "outro", "usage": {"output_tokens": 9}}}),
        json!({"type": "assistant", "requestId": "b", "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": 9}}}),
        json!({"type": "assistant", "requestId": "c", "timestamp": "2026-11-01T10:00:00Z", "message": {"model": "claude-opus-5-5", "usage": {}}}),
    ]
    .map(|v| v.to_string());
    let path = write(&dir, "s.jsonl", &[lines.join("\n"), "não é json".into()]);
    assert!(parse_file(&path, &rules()).is_empty());
}

#[test]
fn cache_com_ttl_separa_escrita_de_5_minutos_e_1_hora() {
    let usage: Dict = serde_json::from_value(json!({
        "cache_creation_input_tokens": 999,
        "cache_creation": {"ephemeral_5m_input_tokens": 1_000_000, "ephemeral_1h_input_tokens": 1_000_000}
    }))
    .unwrap();
    let (total, write) = cost("opus-5-5", &usage).unwrap();
    assert!((write - (4.0 * 1.25 + 4.0 * 2.0)).abs() < 1e-9);
    assert_eq!(total, write);
    let flat: Dict =
        serde_json::from_value(json!({"cache_creation_input_tokens": 1_000_000})).unwrap();
    assert!((cost("opus-5-5", &flat).unwrap().1 - 5.0).abs() < 1e-9);
    assert!(cost("nada", &flat).is_none());
}

#[test]
fn polling_em_primeiro_plano_conta_e_em_segundo_plano_nao() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "s.jsonl",
        &[
            bash_turn("2026-11-01T10:00:00Z", "a", "sleep 90", false),
            bash_turn("2026-11-01T10:01:00Z", "b", "sleep 90", true),
            bash_turn("2026-11-01T10:02:00Z", "c", "ls", false),
        ],
    );
    let rows = parse_file(&path, &rules());
    assert_eq!(rows[0].turns, 3);
    assert_eq!(rows[0].poll_turns, 1);
    assert!((rows[0].poll_cost - (4.0 + 20_000.0) / 1e6).abs() < 1e-12);
}

#[test]
fn acompanhamento_conta_wait_e_despertar_sem_tool() {
    let dir = tempfile::tempdir().unwrap();
    let wake = json!({"type": "user", "message": {"content": [{"type": "text", "text": "<task-notification>x"}]}}).to_string();
    let after = json!({"type": "assistant", "timestamp": "2026-11-01T10:01:00Z", "requestId": "b", "sessionId": "S",
        "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": 1}, "content": []}})
    .to_string();
    let path = write(
        &dir,
        "s.jsonl",
        &[
            bash_turn("2026-11-01T10:00:00Z", "a", "hades wait agents --any", true),
            wake,
            after,
        ],
    );
    let row = &parse_file(&path, &rules())[0];
    assert_eq!(row.follow_turns, Some(2));
}

#[test]
fn reescrita_de_contexto_depois_de_uma_hora_parada() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "s.jsonl",
        &[
            assistant("2026-11-01T10:00:00Z", "a", 1, 0, 50, false),
            assistant("2026-11-01T11:30:00Z", "b", 1, 0, 200_000, false),
            assistant("2026-11-01T11:31:00Z", "c", 1, 0, 200_000, false),
        ],
    );
    let row = &parse_file(&path, &rules())[0];
    assert!((row.rewrite_cost - 200_000.0 * 2.0 * 1.25 / 1e6).abs() < 1e-12);
}

#[test]
fn requisicoes_com_horario_respeitam_o_corte() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "s.jsonl",
        &[
            assistant("2026-11-01T10:00:00Z", "a", 1, 0, 0, false),
            assistant("2026-11-01T10:00:10.500Z", "b", 1, 0, 0, false),
        ],
    );
    let since = parse_ts_ms("2026-11-01T10:00:05Z");
    let (rows, reqs) = parse_file_full(&path, &rules(), since);
    assert_eq!(rows[0].turns, 2);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].epoch_s, 1_793_527_210);
    assert_eq!(reqs[0].model, "sonnet-5-5");
    assert_eq!(reqs[0].tok, [1.0, 5.0, 0.0, 0.0]);
}

#[test]
fn horario_como_o_date_parse() {
    assert_eq!(parse_ts_ms("1970-01-01T00:00:00Z"), 0.0);
    assert_eq!(parse_ts_ms("1970-01-01T00:00:01.5Z"), 1500.0);
    assert_eq!(parse_ts_ms("1970-01-01T01:00:00+01:00"), 0.0);
    assert_eq!(parse_ts_ms("1970-01-02"), 86_400_000.0);
    assert!(parse_ts_ms("t").is_nan());
    assert!(parse_ts_ms("").is_nan());
}

#[test]
fn transcripts_em_qualquer_nivel_em_ordem_sem_seguir_link() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["b.jsonl", "a/c.jsonl", "a/x.txt", "z/d/e.jsonl"] {
        let p = dir.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "").unwrap();
    }
    std::os::unix::fs::symlink(dir.path().join("a"), dir.path().join("link")).unwrap();
    let names: Vec<String> = transcripts_in(dir.path())
        .iter()
        .map(|p| {
            p.strip_prefix(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["a/c.jsonl", "b.jsonl", "z/d/e.jsonl"]);
    assert!(transcripts_in(&dir.path().join("nada")).is_empty());
}

#[test]
fn truthy_e_vazio_seguem_o_js() {
    assert!(truthy(Some(&json!([]))));
    assert!(truthy(Some(&json!({}))));
    assert!(!truthy(Some(&json!(0))));
    assert!(!truthy(Some(&Value::Null)));
    assert!(is_empty(&json!([])));
    assert!(!is_empty(&json!("x")));
}
