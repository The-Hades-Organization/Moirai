//! Configurações do Moirai: as chaves `HADES_MOIRAI_*` do catálogo `config.tsv`, lidas e gravadas só pela API do
//! `hades-core::config`. A página fala com a [`FonteDeConfig`]; o painel do Hades pode trocá-la depois (F6).

use std::path::Path;

use hades_core::Env;
use hades_core::config::{self, ConfigStore, Effective};
use hades_core::paths::{find_state_dir, skill_root};

/// Prefixo das chaves que a página mostra.
pub const PREFIXO: &str = "HADES_MOIRAI_";

/// De onde vale o valor mostrado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origem {
    Padrao,
    Arquivo,
    Ambiente,
}

/// Uma chave com o valor que vale agora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Linha {
    pub chave: String,
    pub grupo: String,
    pub rotulo: String,
    pub descricao: String,
    pub padrao: String,
    pub tipo: String,
    pub valor: String,
    pub origem: Origem,
}

impl Linha {
    /// Faixa do tipo `int:min:max` do catálogo.
    pub fn faixa(&self) -> Option<(i64, i64)> {
        faixa_do_tipo(&self.tipo)
    }

    pub fn somente_leitura(&self) -> bool {
        self.origem == Origem::Ambiente
    }
}

pub fn faixa_do_tipo(tipo: &str) -> Option<(i64, i64)> {
    let mut partes = tipo.strip_prefix("int:")?.split(':');
    let min = partes.next()?.parse().ok()?;
    let max = partes.next()?.parse().ok()?;
    Some((min, max))
}

/// Ponto único por onde a página lê e grava.
pub trait FonteDeConfig {
    fn carregar(&self) -> Result<Vec<Linha>, String>;
    /// Grava `atualizacoes` (já normalizadas) de uma vez; `remover` volta a chave ao padrão.
    fn gravar(&self, atualizacoes: &[(String, String)], remover: &[String]) -> Result<(), String>;
    /// `false` sem instalação do Hades: nada é gravado dentro da skill.
    fn gravavel(&self) -> bool;
}

/// Fonte padrão: o `$D` do `cwd` e o kit da skill, como o CLI.
pub struct ConfigLocal {
    env: Env,
    store: ConfigStore,
    instalado: bool,
}

impl ConfigLocal {
    pub fn new(env: &Env, cwd: &Path) -> Self {
        let kit = skill_root(env).join("kit");
        let estado = find_state_dir(env, cwd);
        Self {
            instalado: estado.is_some(),
            store: ConfigStore {
                dir: estado.unwrap_or_else(|| kit.clone()),
                kit,
            },
            env: env.clone(),
        }
    }

    pub fn com_store(env: &Env, store: ConfigStore) -> Self {
        Self {
            env: env.clone(),
            store,
            instalado: true,
        }
    }
}

impl FonteDeConfig for ConfigLocal {
    fn carregar(&self) -> Result<Vec<Linha>, String> {
        let efetivas = config::effective(&self.store).map_err(|e| e.0)?;
        Ok(efetivas
            .into_iter()
            .filter(|e| e.item.key.starts_with(PREFIXO))
            .map(|e| self.linha(e))
            .collect())
    }

    fn gravar(&self, atualizacoes: &[(String, String)], remover: &[String]) -> Result<(), String> {
        if !self.instalado {
            return Err("sem instalação do Hades: nada a gravar".to_owned());
        }
        let pares: Vec<(&str, &str)> = atualizacoes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let remover: Vec<&str> = remover.iter().map(String::as_str).collect();
        config::save(&self.store, &pares, &remover)
            .map(|_| ())
            .map_err(|e| e.0)
    }

    fn gravavel(&self) -> bool {
        self.instalado
    }
}

impl ConfigLocal {
    fn linha(&self, e: Effective) -> Linha {
        let (valor, origem) = match self.env.get_raw(&e.item.key) {
            Some(v) => (v.to_owned(), Origem::Ambiente),
            None if e.changed => (e.value, Origem::Arquivo),
            None => (e.value, Origem::Padrao),
        };
        Linha {
            chave: e.item.key,
            grupo: e.item.group,
            rotulo: e.item.label,
            descricao: e.item.desc,
            padrao: e.item.default,
            tipo: e.item.kind,
            valor,
            origem,
        }
    }
}

/// `validate` do TS (`config-catalog.ts:138`) para `int:min:max`: inteiro inteiro (`-?[0-9]+`) dentro da faixa,
/// normalizado sem zeros à esquerda. Outros tipos só passam aparados.
pub fn validar(linha: &Linha, texto: &str) -> Result<String, String> {
    let t = texto.trim();
    let Some((min, max)) = linha.faixa() else {
        return Ok(t.to_owned());
    };
    let corpo = t.strip_prefix('-').unwrap_or(t);
    let valido = !corpo.is_empty() && corpo.bytes().all(|b| b.is_ascii_digit());
    let n = valido.then(|| t.parse::<i128>().ok()).flatten();
    match n {
        Some(n) if (i128::from(min)..=i128::from(max)).contains(&n) => Ok(n.to_string()),
        _ => Err(format!("Informe um número inteiro entre {min} e {max}")),
    }
}

/// Quantas linhas editáveis têm texto diferente do valor que vale.
pub fn pendentes(linhas: &[Linha], textos: &[String]) -> usize {
    linhas
        .iter()
        .zip(textos)
        .filter(|(l, t)| !l.somente_leitura() && t.trim() != l.valor)
        .count()
}

/// Erro em linha de cada texto (ambiente nunca tem erro: é só leitura).
pub fn erros(linhas: &[Linha], textos: &[String]) -> Vec<Option<String>> {
    linhas
        .iter()
        .zip(textos)
        .map(|(l, t)| {
            if l.somente_leitura() {
                None
            } else {
                validar(l, t).err()
            }
        })
        .collect()
}

/// Grava as linhas alteradas. Recusa tudo se qualquer uma for inválida; devolve quantas gravou.
pub fn salvar(
    fonte: &dyn FonteDeConfig,
    linhas: &[Linha],
    textos: &[String],
) -> Result<usize, String> {
    if !fonte.gravavel() {
        return Err("Sem instalação do Hades: nada a gravar".to_owned());
    }
    let mut atualizar = Vec::new();
    for (l, t) in linhas.iter().zip(textos) {
        if l.somente_leitura() || t.trim() == l.valor {
            continue;
        }
        let valor = validar(l, t).map_err(|e| format!("{}: {e}", l.rotulo))?;
        if valor != l.valor {
            atualizar.push((l.chave.clone(), valor));
        }
    }
    if atualizar.is_empty() {
        return Ok(0);
    }
    fonte.gravar(&atualizar, &[])?;
    Ok(atualizar.len())
}

#[cfg(test)]
mod tests;
