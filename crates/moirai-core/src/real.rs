//! Custo real corrente de um worktree por offset de transcript (`src/commands/moirai.ts:100-157`,
//! `src/lib/moirai.ts:542-583`).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hades_core::Env;
use serde_json::{Map, Number, Value, json};

use crate::price::model_key;
use crate::transcripts::{cost, transcripts_in};

/// Custo já lido de um transcript: até onde (bytes), quanto custou e os pedidos já somados (um pedido tem várias
/// linhas) (`FileCost`, `moirai.ts:547`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileCost {
    pub offset: u64,
    pub cost_usd: f64,
    pub ids: Vec<String>,
}

impl FileCost {
    /// Forma serializada do cache do watch: `{"offset", "costUsd", "ids"}`.
    pub fn to_value(&self) -> Value {
        json!({
            "offset": self.offset,
            "costUsd": Number::from_f64(self.cost_usd).map_or(Value::Null, Value::Number),
            "ids": self.ids,
        })
    }

    /// `validFileCost` (`moirai.ts:57`): offset inteiro não negativo, `costUsd` número e `ids` lista.
    pub fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        let offset = map.get("offset")?.as_f64()?;
        if offset < 0.0 || offset.fract() != 0.0 || !offset.is_finite() {
            return None;
        }
        let cost_usd = map.get("costUsd")?.as_f64()?;
        let ids = map.get("ids")?.as_array()?;
        Some(Self {
            offset: offset as u64,
            cost_usd,
            ids: ids
                .iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        })
    }
}

/// Por arquivo; lido de disco pelo chamador, então cada entrada é conferida antes de valer (`CostCache`).
pub type CostCache = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurrentCost {
    pub cost_usd: f64,
    /// Bytes novos lidos nesta chamada.
    pub read: u64,
}

/// `<home>/.claude/projects`.
pub fn default_projects_dir(env: &Env) -> PathBuf {
    env.home().join(".claude").join("projects")
}

/// Troca todo não alfanumérico por `-`, uma vez por unidade UTF-16 como o `replace(/[^A-Za-z0-9]/g, "-")`.
pub fn encode_cwd(cwd: &str) -> String {
    cwd.chars()
        .flat_map(|c| {
            let n = if c.is_ascii_alphanumeric() {
                0
            } else {
                c.len_utf16()
            };
            if n == 0 { vec![c] } else { vec!['-'; n] }
        })
        .collect()
}

fn str_of(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// Soma ao `prev` o custo das linhas completas de `text` (o acréscimo do transcript). Um pedido conta uma vez pelo
/// `requestId` (ou `message.id`), com o `usage` da primeira linha; sem modelo conhecido ou sem horário não custa.
/// `bytes` é quanto o texto consumiu (`addTranscriptText`, `moirai.ts:561`).
pub fn add_transcript_text(prev: &FileCost, text: &str, bytes: u64) -> FileCost {
    let mut seen: std::collections::HashSet<String> = prev.ids.iter().cloned().collect();
    let mut cost_usd = prev.cost_usd;
    let mut ids = prev.ids.clone();
    let empty = Map::new();
    for raw in text.split('\n') {
        if !raw.contains("\"assistant\"") {
            continue;
        }
        let Ok(Value::Object(row)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if row.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let message = row
            .get("message")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let Some(usage) = message
            .get("usage")
            .and_then(Value::as_object)
            .filter(|u| !u.is_empty())
        else {
            continue;
        };
        let Some(key) = model_key(message.get("model").and_then(Value::as_str)) else {
            continue;
        };
        if str_of(row.get("timestamp")).is_empty() {
            continue;
        }
        let request_id = str_of(row.get("requestId"));
        let id = if request_id.is_empty() {
            str_of(message.get("id"))
        } else {
            request_id
        };
        if !id.is_empty() {
            if !seen.insert(id.to_owned()) {
                continue;
            }
            ids.push(id.to_owned());
        }
        if let Some((total, _)) = cost(key, usage) {
            cost_usd += total;
        }
    }
    FileCost {
        offset: prev.offset + bytes,
        cost_usd,
        ids,
    }
}

/// Lê de `offset` até o fim do arquivo e devolve só as linhas completas; o que o agente ainda escreve fica para o
/// próximo ciclo (`readIncrement`, `moirai.ts:119`).
fn read_increment(path: &Path, offset: u64, size: u64) -> Option<(String, u64)> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buffer = Vec::new();
    file.take(size - offset).read_to_end(&mut buffer).ok()?;
    let end = buffer
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    Some((
        String::from_utf8_lossy(&buffer[..end]).into_owned(),
        end as u64,
    ))
}

/// Custo real corrente de um worktree, somando os transcripts da pasta do projeto no Claude. O cache guarda o offset
/// já lido por arquivo: cada chamada lê só o acréscimo; arquivo que encolheu recomeça do zero (`currentCost`,
/// `moirai.ts:132`).
pub fn current_cost(worktree: &str, cache: &mut CostCache, projects_dir: &Path) -> CurrentCost {
    if worktree.is_empty() {
        return CurrentCost {
            cost_usd: 0.0,
            read: 0,
        };
    }
    let mut read = 0;
    let mut cost_usd = 0.0;
    for path in transcripts_in(&projects_dir.join(encode_cwd(worktree))) {
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let size = meta.len();
        let key = path.to_string_lossy().into_owned();
        let mut state = cache
            .get(&key)
            .and_then(FileCost::from_value)
            .unwrap_or_default();
        if size < state.offset {
            state = FileCost::default();
        }
        if size > state.offset
            && let Some((text, bytes)) = read_increment(&path, state.offset, size)
        {
            state = add_transcript_text(&state, &text, bytes);
            read += bytes;
        }
        cost_usd += state.cost_usd;
        cache.insert(key, state.to_value());
    }
    CurrentCost { cost_usd, read }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn line(id: &str, output_tokens: u64) -> String {
        format!(
            "{}\n",
            json!({"type": "assistant", "requestId": id, "timestamp": "2026-10-01T10:00:00Z",
                "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": output_tokens}}})
        )
    }

    struct Rig {
        _dir: tempfile::TempDir,
        worktree: String,
        projects: PathBuf,
        file: PathBuf,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let worktree = format!("{}/wt-cost", dir.path().display());
        let projects = dir.path().join("projects");
        let folder = projects.join(encode_cwd(&worktree));
        fs::create_dir_all(&folder).unwrap();
        let file = folder.join("s.jsonl");
        Rig {
            _dir: dir,
            worktree,
            projects,
            file,
        }
    }

    fn append(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn primeira_leitura_soma_tudo_100k_de_saida_sao_2_dolares() {
        let r = rig();
        let mut cache = CostCache::new();
        fs::write(&r.file, line("a", 100_000)).unwrap();
        let first = current_cost(&r.worktree, &mut cache, &r.projects);
        assert!((first.cost_usd - 2.0).abs() < 1e-6);
        assert!(first.read > 0);
    }

    #[test]
    fn sem_novidade_nao_le_byte_nenhum_e_o_custo_fica() {
        let r = rig();
        let mut cache = CostCache::new();
        fs::write(&r.file, line("a", 100_000)).unwrap();
        current_cost(&r.worktree, &mut cache, &r.projects);
        let again = current_cost(&r.worktree, &mut cache, &r.projects);
        assert_eq!(again.read, 0);
        assert!((again.cost_usd - 2.0).abs() < 1e-6);
    }

    #[test]
    fn acrescimo_le_so_ele_e_o_mesmo_requestid_nao_soma_de_novo() {
        let r = rig();
        let mut cache = CostCache::new();
        fs::write(&r.file, line("a", 100_000)).unwrap();
        current_cost(&r.worktree, &mut cache, &r.projects);
        let add = line("b", 50_000) + &line("b", 50_000);
        append(&r.file, &add);
        let next = current_cost(&r.worktree, &mut cache, &r.projects);
        assert_eq!(next.read, add.len() as u64);
        assert!((next.cost_usd - 3.0).abs() < 1e-6);
    }

    #[test]
    fn linha_incompleta_fica_para_o_proximo_ciclo() {
        let r = rig();
        let mut cache = CostCache::new();
        fs::write(&r.file, line("a", 100_000)).unwrap();
        current_cost(&r.worktree, &mut cache, &r.projects);
        let whole = line("c", 50_000);
        append(&r.file, &whole[..40]);
        assert_eq!(current_cost(&r.worktree, &mut cache, &r.projects).read, 0);
        append(&r.file, &whole[40..]);
        let done = current_cost(&r.worktree, &mut cache, &r.projects);
        assert_eq!(done.read, whole.len() as u64);
        assert!((done.cost_usd - 3.0).abs() < 1e-6);
    }

    #[test]
    fn arquivo_que_encolheu_recomeca_do_zero() {
        let r = rig();
        let mut cache = CostCache::new();
        fs::write(&r.file, line("a", 100_000) + &line("b", 50_000)).unwrap();
        current_cost(&r.worktree, &mut cache, &r.projects);
        fs::write(&r.file, line("z", 25_000)).unwrap();
        let after = current_cost(&r.worktree, &mut cache, &r.projects);
        assert!((after.cost_usd - 0.5).abs() < 1e-6);
    }

    #[test]
    fn cache_corrompido_no_disco_e_descartado_e_relido() {
        let r = rig();
        fs::write(&r.file, line("z", 25_000)).unwrap();
        let mut bad = CostCache::new();
        bad.insert(
            r.file.to_string_lossy().into_owned(),
            json!({"offset": "x"}),
        );
        let cost = current_cost(&r.worktree, &mut bad, &r.projects);
        assert!((cost.cost_usd - 0.5).abs() < 1e-6);
        let stored = &bad[r.file.to_string_lossy().as_ref()];
        assert_eq!(FileCost::from_value(stored).unwrap().ids, ["z"]);
    }

    #[test]
    fn worktree_vazio_ou_sem_transcript_custa_zero() {
        let r = rig();
        let zero = CurrentCost {
            cost_usd: 0.0,
            read: 0,
        };
        assert_eq!(current_cost("", &mut CostCache::new(), &r.projects), zero);
        assert_eq!(
            current_cost("/nada/aqui", &mut CostCache::new(), &r.projects),
            zero
        );
    }

    #[test]
    fn modelo_desconhecido_ou_sem_horario_nao_custa() {
        let odd = format!(
            "{}\n{}\nnão é json\n",
            json!({"type": "assistant", "requestId": "q", "timestamp": "t", "message": {"model": "outro", "usage": {"output_tokens": 9e9}}}),
            json!({"type": "assistant", "requestId": "p", "message": {"model": "claude-opus-5-5", "usage": {"output_tokens": 9e9}}}),
        );
        let got = add_transcript_text(&FileCost::default(), &odd, odd.len() as u64);
        assert_eq!(got.cost_usd, 0.0);
        assert_eq!(got.offset, odd.len() as u64);
    }

    #[test]
    fn message_id_vale_quando_nao_ha_requestid() {
        let l = |id: &str| {
            format!(
                "{}\n",
                json!({"type": "assistant", "timestamp": "t",
                    "message": {"id": id, "model": "claude-opus-5-5", "usage": {"output_tokens": 50_000}}})
            )
        };
        let text = l("m1") + &l("m1");
        let got = add_transcript_text(&FileCost::default(), &text, text.len() as u64);
        assert!((got.cost_usd - 1.0).abs() < 1e-6);
        assert_eq!(got.ids, ["m1"]);
    }

    #[test]
    fn cache_serializado_guarda_offset_custo_e_ids() {
        let state = FileCost {
            offset: 7,
            cost_usd: 2.0,
            ids: vec!["a".into()],
        };
        let v = state.to_value();
        assert_eq!(
            hades_core::jsonfmt::json_compact(&v),
            r#"{"offset":7,"costUsd":2,"ids":["a"]}"#
        );
        assert_eq!(FileCost::from_value(&v), Some(state));
        assert_eq!(
            FileCost::from_value(&json!({"offset": 3.0, "costUsd": 1, "ids": []}))
                .unwrap()
                .offset,
            3
        );
        assert!(FileCost::from_value(&json!({"offset": -1, "costUsd": 1, "ids": []})).is_none());
        assert!(FileCost::from_value(&json!({"offset": 1.5, "costUsd": 1, "ids": []})).is_none());
        assert!(FileCost::from_value(&json!({"offset": 1, "costUsd": "x", "ids": []})).is_none());
    }

    #[test]
    fn pasta_do_projeto_troca_nao_alfanumerico_por_traco() {
        assert_eq!(encode_cwd("/home/a_b.c"), "-home-a-b-c");
        assert_eq!(encode_cwd("é😀"), "---");
    }
}
