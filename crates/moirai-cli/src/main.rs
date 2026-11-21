//! `moirai`: argumentos e impressão; a regra mora no `moirai-core` (`src/commands/moirai.ts`).

mod deep;

use std::path::PathBuf;
use std::process::ExitCode;

use hades_core::Env;
use hades_core::fsx::{is_file, read_text, realpath_missing_ok};
use hades_core::jsonfmt::json_indent;
use hades_core::output::{HadesError, exit, out, warn};
use hades_core::paths::hades_home;
use moirai_core::deep::deep_hints;
use moirai_core::estimate::estimate_plan;
use moirai_core::history::{clock_ms, collect_history, history_json, history_lines, window_days};
use moirai_core::limits::{limites_do_plano, limits_now};
use moirai_core::model::Mode;
use moirai_core::params::load_params;
use moirai_core::plan::{SectionInput, has_section, plan_units, render_section, write_section};
use moirai_core::report::{costs_of, estimate_json_value, estimate_text};
use std::collections::HashMap;

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

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` de `Date.prototype.toISOString`.
fn iso_millis(ms: i64) -> String {
    let secs = hades_core::registry::iso_from_epoch(ms.div_euclid(1000));
    format!("{}.{:03}Z", &secs[..secs.len() - 1], ms.rem_euclid(1000))
}

/// `estimate` (`src/commands/moirai.ts:301`).
fn estimate(args: &[String], env: &Env, cwd: &std::path::Path) -> Result<i32, HadesError> {
    let mut plan = String::new();
    let mut mode: Option<Mode> = None;
    let (mut write, mut json, mut deep) = (false, false, false);
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--mode" {
            i += 1;
            mode = match args.get(i).map(String::as_str) {
                Some("skill") => Some(Mode::Skill),
                Some("projeto") => Some(Mode::Projeto),
                _ => {
                    return Err(HadesError::new(format!(
                        "--mode: skill ou projeto\n{USAGE}"
                    )));
                }
            };
        } else if arg == "--write" {
            write = true;
        } else if arg == "--json" {
            json = true;
        } else if arg == "--deep" {
            deep = true;
        } else if arg.starts_with('-') || !plan.is_empty() {
            return Err(HadesError::new(format!(
                "argumento desconhecido: {arg}\n{USAGE}"
            )));
        } else {
            plan = arg.to_owned();
        }
        i += 1;
    }
    if plan.is_empty() {
        return Err(HadesError::new(format!("faltou o plan.md\n{USAGE}")));
    }
    let plan_arg = std::path::Path::new(&plan);
    if !is_file(plan_arg) {
        return Err(HadesError::new(format!("plano não encontrado: {plan}")));
    }
    let path = realpath_missing_ok(plan_arg);
    let text = read_text(&path).unwrap_or_default();
    let units = plan_units(&text);
    let path_text = path.to_string_lossy().into_owned();
    if units.is_empty() {
        return Err(HadesError::new(format!(
            "{path_text}: sem tabela em \"## Unidades\" (colunas Unidade, Papel e Escopo ou Specs); nada a estimar"
        )));
    }
    let plans_dir = format!("{}/", hades_home(env).join("plans").to_string_lossy());
    let plan_mode = mode.unwrap_or(if path_text.starts_with(&plans_dir) {
        Mode::Skill
    } else {
        Mode::Projeto
    });
    let params = load_params(env, cwd)?;
    let history = collect_history(env, cwd, &params)?;
    let hints = if deep {
        let runner = deep::HadesRunner::new(env)?;
        deep_hints(&units, &mut |prompt| runner.estimate(prompt))?
    } else {
        HashMap::new()
    };
    let estimates = estimate_plan(&units, plan_mode, &history, &params, &hints);
    let now = clock_ms(env)?;
    if write {
        let iso = iso_millis(now);
        let section = render_section(&SectionInput {
            date: &iso[..10],
            estimates: &estimates,
            history_size: history.len(),
            window_days: window_days(env, cwd)?,
            params,
        });
        let updated = write_section(&text, &section);
        let replaced = has_section(&text);
        hades_core::atomic::write_file_atomic(&path, updated.as_bytes())
            .map_err(|e| HadesError::new(format!("{path_text}: {e}")))?;
        warn(&format!(
            "moirai: seção \"## Estimativa\" gravada em {path_text}{}; está dentro do hash da aprovação: aprove de novo (hades agent plan-approve)",
            if replaced {
                " (substituiu a anterior)"
            } else {
                ""
            }
        ));
    }
    let measured = limits_now(env, cwd)?;
    let limits = limites_do_plano(&costs_of(&estimates), &measured.calibracao, &measured.atual);
    if json {
        let value = estimate_json_value(
            &path_text,
            &iso_millis(now),
            &estimates,
            &params,
            history.len(),
            &limits,
        );
        out(&json_indent(&value, 1));
    } else {
        out(&estimate_text(&path_text, &estimates, &limits).join("\n"));
    }
    Ok(exit::OK)
}

/// `moiraiCommand` (`src/commands/moirai.ts:339`).
fn run(args: &[String], env: &Env, cwd: &std::path::Path) -> Result<i32, HadesError> {
    match args.first().map(String::as_str) {
        Some("history") => history(&args[1..], env, cwd),
        Some("estimate") => estimate(&args[1..], env, cwd),
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
