//! Expressões de polling do hook `hades guard`, lidas de `loop/hooks/guard-rules.json` (`src/lib/guard-rules.ts`).
//!
//! O arquivo usa lookahead e backreference (`heredoc`), por isso `fancy-regex`. `\w`, `\d` e `\b` são reescritos para
//! ASCII (`ascii_classes`), como no JS.

use fancy_regex::{Captures, Regex};
use hades_core::{Env, fsx::read_text, paths::skill_root};
use serde_json::Value;

const EMBEDDED: &str = include_str!("../../data/guard-rules.json");

pub struct Rule {
    pub rx: Regex,
    pub what: String,
}

pub struct GuardRules {
    pub fg_poll: Vec<Rule>,
    /// Checagens que o hook só barra na repetição.
    pub repeat: Vec<Rule>,
    pub checks: Regex,
    pub agent_text: Regex,
    pub quoted: Regex,
    pub heredoc: Regex,
}

const WORD_BOUNDARY: &str =
    "(?:(?<=[0-9A-Za-z_])(?![0-9A-Za-z_])|(?<![0-9A-Za-z_])(?=[0-9A-Za-z_]))";
const NOT_WORD_BOUNDARY: &str =
    "(?:(?<=[0-9A-Za-z_])(?=[0-9A-Za-z_])|(?<![0-9A-Za-z_])(?![0-9A-Za-z_]))";

/// `\w`, `\d`, `\W`, `\D`, `\b` e `\B` com o sentido ASCII do JS (o regex do Rust os faz Unicode).
fn ascii_classes(re: &str) -> String {
    let mut out = String::with_capacity(re.len());
    let mut in_class = false;
    let mut chars = re.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('w') if in_class => out.push_str("0-9A-Za-z_"),
                Some('d') if in_class => out.push_str("0-9"),
                Some('w') => out.push_str("[0-9A-Za-z_]"),
                Some('W') if !in_class => out.push_str("[^0-9A-Za-z_]"),
                Some('d') => out.push_str("[0-9]"),
                Some('D') if !in_class => out.push_str("[^0-9]"),
                Some('b') if !in_class => out.push_str(WORD_BOUNDARY),
                Some('B') if !in_class => out.push_str(NOT_WORD_BOUNDARY),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '[' if !in_class => {
                in_class = true;
                out.push(c);
                if let Some(next) = chars.clone().next() {
                    if next == '^' {
                        out.push('^');
                        chars.next();
                    }
                    if chars.clone().next() == Some(']') {
                        out.push_str("\\]");
                        chars.next();
                    }
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn compile(spec: &Value) -> Result<(Regex, String), String> {
    let re = spec
        .get("re")
        .and_then(Value::as_str)
        .ok_or("guard-rules: regra sem `re`")?;
    let flags = spec.get("flags").and_then(Value::as_str).unwrap_or("");
    let prefix = if flags.contains('s') { "(?s)" } else { "" };
    let rx = Regex::new(&format!("{prefix}{}", ascii_classes(re)))
        .map_err(|e| format!("guard-rules: {e}"))?;
    let what = spec
        .get("what")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok((rx, what))
}

fn rules_of(value: &Value, key: &str) -> Result<Vec<Rule>, String> {
    let list = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("guard-rules: `{key}` ausente"))?;
    list.iter()
        .map(|spec| compile(spec).map(|(rx, what)| Rule { rx, what }))
        .collect()
}

fn one(value: &Value, key: &str) -> Result<Regex, String> {
    let spec = value
        .get(key)
        .ok_or_else(|| format!("guard-rules: `{key}` ausente"))?;
    compile(spec).map(|(rx, _)| rx)
}

impl GuardRules {
    /// `guardRules(file)` (`guard-rules.ts:44`) sobre o texto do arquivo.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let raw: Value = serde_json::from_str(text).map_err(|e| format!("guard-rules: {e}"))?;
        Ok(Self {
            fg_poll: rules_of(&raw, "fgPoll")?,
            repeat: rules_of(&raw, "repeat")?,
            checks: one(&raw, "usageChecks")?,
            agent_text: one(&raw, "agentText")?,
            quoted: one(&raw, "quoted")?,
            heredoc: one(&raw, "heredoc")?,
        })
    }

    /// A cópia embutida, igual à da skill no momento da compilação.
    pub fn embedded() -> Self {
        Self::from_json(EMBEDDED).expect("guard-rules embutido é válido")
    }

    /// `<skill_root>/loop/hooks/guard-rules.json`; sem o arquivo, a cópia embutida.
    pub fn load(env: &Env) -> Result<Self, String> {
        let file = skill_root(env)
            .join("loop")
            .join("hooks")
            .join("guard-rules.json");
        match read_text(&file) {
            Some(text) => Self::from_json(&text),
            None => Ok(Self::embedded()),
        }
    }

    /// `hades agent answer|prompt|…`: o que está entre aspas é dado, não comando; aspas com `$(` ou crase executam e
    /// ficam (`stripAgentText`, `guard-rules.ts:57`).
    pub fn strip_agent_text(&self, cmd: &str) -> String {
        self.agent_text
            .replace_all(cmd, |caps: &Captures| {
                let head = caps.get(1).map_or("", |m| m.as_str());
                let args = caps.get(2).map_or("", |m| m.as_str());
                let args = self.quoted.replace_all(args, |quoted: &Captures| {
                    let text = quoted.get(0).map_or("", |m| m.as_str());
                    if text.contains("$(") || text.contains('`') {
                        text.to_owned()
                    } else {
                        "\"\"".to_owned()
                    }
                });
                format!("{head}{args}")
            })
            .into_owned()
    }

    /// Corpo de heredoc é dado: fica só a linha de abertura (`stripHeredocBodies`, `guard-rules.ts:64`).
    pub fn strip_heredoc_bodies(&self, cmd: &str) -> String {
        self.heredoc
            .replace_all(cmd, |caps: &Captures| {
                caps.get(1).map_or("", |m| m.as_str()).to_owned()
            })
            .into_owned()
    }

    /// Comando de espera em primeiro plano (`isPolling`, `guard-rules.ts:69`).
    pub fn is_polling(&self, cmd: &str) -> bool {
        let text = self.strip_heredoc_bodies(&self.strip_agent_text(cmd));
        self.checks.is_match(&text).unwrap_or(false)
            || self
                .fg_poll
                .iter()
                .any(|rule| rule.rx.is_match(&text).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> GuardRules {
        GuardRules::embedded()
    }

    #[test]
    fn classes_seguem_o_ascii_do_js() {
        assert_eq!(
            ascii_classes("\\bsleep\\s+\\d+"),
            format!("{WORD_BOUNDARY}sleep\\s+[0-9]+")
        );
        assert_eq!(ascii_classes("[\\w.-]\\W"), "[0-9A-Za-z_.-][^0-9A-Za-z_]");
        let r = rules();
        assert!(!r.is_polling("xsleep 60"));
        assert!(r.is_polling("sleep 60"));
        assert!(r.is_polling("tail -f x"));
        assert!(!r.is_polling("tail -é x"));
        assert!(!r.is_polling("sleep ٦٠"));
        assert!(r.is_polling("sleep 60é"));
    }

    #[test]
    fn heredoc_com_sleep_longo_nao_e_polling_fora_dele_e() {
        let r = rules();
        assert!(!r.is_polling("cat > f <<'EOF'\nsleep 60\nEOF"));
        assert!(r.is_polling("sleep 60"));
    }

    #[test]
    fn texto_entre_aspas_de_agent_answer_vira_dado_com_substituicao_fica() {
        let r = rules();
        assert_eq!(
            r.strip_agent_text("hades-agent.sh answer w \"sleep 90\" && ls"),
            "hades-agent.sh answer w \"\" && ls"
        );
        assert_eq!(
            r.strip_agent_text("hades-agent.sh answer w \"$(sleep 90)\""),
            "hades-agent.sh answer w \"$(sleep 90)\""
        );
    }

    #[test]
    fn heredoc_so_a_linha_de_abertura_fica() {
        assert_eq!(
            rules().strip_heredoc_bodies("cat <<'EOF' > f\ncorpo\nEOF"),
            "cat <<'EOF' > f\n"
        );
    }

    #[test]
    fn checagens_so_barradas_na_repeticao_contam_como_polling_no_consumo() {
        let r = rules();
        for cmd in [
            "gh pr checks 12",
            "gh run view 9",
            "herdr pane list",
            "~/x/gate-lock.sh status",
        ] {
            assert!(r.is_polling(cmd), "{cmd}");
        }
        for cmd in [
            "sleep 4 && curl x",
            "git log | head -5",
            "npm test 2>&1 | tail -40",
        ] {
            assert!(!r.is_polling(cmd), "{cmd}");
        }
    }

    #[test]
    fn rotulos_das_regras_vem_do_arquivo() {
        let r = rules();
        assert_eq!(r.fg_poll[0].what, "sleep de espera");
        assert_eq!(r.repeat.len(), 6);
    }

    #[test]
    fn arquivo_invalido_e_recusado() {
        assert!(GuardRules::from_json("{").is_err());
        assert!(GuardRules::from_json("{}").is_err());
    }

    #[test]
    fn sem_o_arquivo_da_skill_vale_a_copia_embutida() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::empty().with("HADES_SKILL_DIR", dir.path().to_str().unwrap());
        assert!(GuardRules::load(&env).unwrap().is_polling("sleep 60"));
    }
}
