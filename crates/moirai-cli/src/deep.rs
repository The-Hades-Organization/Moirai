//! Runner do `--deep`: pede o argv ao `hades provider print-argv` e roda o estimador
//! (`claudeRunner`, `src/commands/moirai.ts:213`). O provedor ativo, o modelo, o binário e o timeout vêm do Hades.

use std::path::PathBuf;
use std::time::Duration;

use hades_core::Env;
use hades_core::output::{HadesError, warn};
use hades_core::proc::{RunOptions, run, which};
use serde_json::Value;

/// Binário do Hades; ausente, procura `hades` no PATH.
pub const HADES_BIN_ENV: &str = "MOIRAI_HADES_BIN";

const MAX_BUFFER: usize = 8 << 20;

/// Saída de `hades provider print-argv`.
#[derive(Debug, PartialEq, Eq)]
pub struct PrintArgv {
    pub bin: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout_s: u64,
}

pub fn parse_print_argv(stdout: &str) -> Result<PrintArgv, String> {
    let value: Value =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("JSON inválido: {e}"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let bin = text("bin").ok_or("faltou \"bin\"")?;
    let cwd = text("cwd").ok_or("faltou \"cwd\"")?;
    let args = value
        .get("args")
        .and_then(Value::as_array)
        .ok_or("faltou \"args\"")?
        .iter()
        .map(|a| {
            a.as_str()
                .map(str::to_owned)
                .ok_or("\"args\" só aceita texto")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let timeout_s = value
        .get("timeout_s")
        .and_then(Value::as_u64)
        .ok_or("faltou \"timeout_s\"")?
        .max(1);
    Ok(PrintArgv {
        bin,
        args,
        cwd: PathBuf::from(cwd),
        timeout_s,
    })
}

pub struct HadesRunner {
    hades: String,
}

impl HadesRunner {
    /// Falha com código 2 se não houver `hades`.
    pub fn new(env: &Env) -> Result<Self, HadesError> {
        let hades = match env.get(HADES_BIN_ENV).filter(|v| !v.is_empty()) {
            Some(path) => path.to_owned(),
            None => which("hades", env.get("PATH").unwrap_or(""))
                .ok_or_else(|| {
                    HadesError::new(
                        "moirai --deep: o comando `hades` não está no PATH; ele informa o provedor e o modelo do estimador",
                    )
                })?
                .to_string_lossy()
                .into_owned(),
        };
        Ok(Self { hades })
    }

    fn argv(&self, prompt: &str) -> Result<PrintArgv, HadesError> {
        let fail = |why: String| {
            HadesError::new(format!(
                "moirai --deep: `hades provider print-argv` falhou: {why}"
            ))
        };
        let r = run(
            &self.hades,
            &[
                "provider",
                "print-argv",
                "--role",
                "ORCHESTRATOR",
                "--read-only",
                "--allow-tools",
                "Read Grep Glob",
                "--prompt",
                prompt,
            ],
            &RunOptions {
                max_buffer: Some(MAX_BUFFER),
                ..RunOptions::default()
            },
        );
        if r.status != 0 {
            let stderr = r.stderr.trim();
            return Err(fail(format!(
                "saiu {}{}",
                r.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            )));
        }
        parse_print_argv(&r.stdout).map_err(fail)
    }

    /// Roda o estimador de uma unidade: o stdout se saiu 0, `None` se não.
    pub fn estimate(&self, prompt: &str) -> Result<Option<String>, HadesError> {
        let spec = self.argv(prompt)?;
        let args: Vec<&str> = spec.args.iter().map(String::as_str).collect();
        let r = run(
            &spec.bin,
            &args,
            &RunOptions {
                cwd: Some(spec.cwd),
                timeout: Some(Duration::from_secs(spec.timeout_s)),
                max_buffer: Some(MAX_BUFFER),
            },
        );
        if r.timed_out {
            warn(&format!(
                "moirai --deep: o estimador passou de {}s (HADES_MOIRAI_DEEP_TIMEOUT_S)",
                spec.timeout_s
            ));
        }
        Ok((r.status == 0).then_some(r.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_a_linha_json_do_hades() {
        let got = parse_print_argv(
            "{\"bin\":\"claude\",\"args\":[\"-p\",\"x\"],\"cwd\":\"/s\",\"timeout_s\":0}\n",
        )
        .unwrap();
        assert_eq!(got.bin, "claude");
        assert_eq!(got.args, ["-p", "x"]);
        assert_eq!(got.cwd, PathBuf::from("/s"));
        assert_eq!(got.timeout_s, 1);
    }

    #[test]
    fn recusa_saida_incompleta_ou_torta() {
        assert!(parse_print_argv("nao json").is_err());
        assert!(
            parse_print_argv("{\"bin\":\"c\",\"args\":[1],\"cwd\":\"/\",\"timeout_s\":3}").is_err()
        );
        assert!(parse_print_argv("{\"bin\":\"c\",\"args\":[],\"cwd\":\"/\"}").is_err());
    }

    #[test]
    fn sem_hades_no_path_o_erro_e_de_uso() {
        let env = Env::empty().with("PATH", "/nao-existe");
        let err = HadesRunner::new(&env).err().unwrap();
        assert_eq!(err.code, 2);
        assert!(err.message.contains("`hades` não está no PATH"));
    }
}
