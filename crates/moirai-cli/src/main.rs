//! `moirai`: argumentos e impressão; a regra mora no `moirai-core` (`src/commands/moirai.ts`).

use std::path::PathBuf;
use std::process::ExitCode;

use hades_core::Env;
use hades_core::output::{HadesError, exit, out, warn};
use moirai_core::history::{collect_history, history_json, history_lines};
use moirai_core::params::load_params;

const USAGE: &str = "uso: hades moirai history [--json]
     hades moirai estimate <plan.md> [--write] [--deep] [--json] [--mode skill|projeto]
  history    uma linha por unidade encerrada (custo, tokens, relógio, gate, clears, tamanho real)
  estimate   mediana p25–p75 por unidade do plano; --write grava a seção \"## Estimativa\" no plano (entra no hash da
             aprovação); --deep abre um subagente (claude -p) que estima o tamanho do escopo, só a pedido";

/// `history` (`src/commands/moirai.ts:263`).
fn history(args: &[String], env: &Env, cwd: &std::path::Path) -> Result<i32, HadesError> {
    let json = args.iter().any(|a| a == "--json");
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--json")
        .collect();
    if !unknown.is_empty() {
        return Err(HadesError::new(format!(
            "argumento desconhecido: {}\n{USAGE}",
            unknown.join(" ")
        )));
    }
    let params = load_params(env, cwd)?;
    let units = collect_history(env, cwd, &params)?;
    if json {
        out(&history_json(&units));
    } else {
        out(&history_lines(&units).join("\n"));
    }
    Ok(exit::OK)
}

/// `moiraiCommand` (`src/commands/moirai.ts:339`).
fn run(args: &[String], env: &Env, cwd: &std::path::Path) -> Result<i32, HadesError> {
    match args.first().map(String::as_str) {
        Some("history") => history(&args[1..], env, cwd),
        sub => {
            out(USAGE);
            Ok(if matches!(sub, Some("-h" | "--help")) {
                exit::OK
            } else {
                exit::USAGE
            })
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match run(&args, &Env::from_process(), &cwd) {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            warn(&error.message);
            ExitCode::from(error.code as u8)
        }
    }
}
