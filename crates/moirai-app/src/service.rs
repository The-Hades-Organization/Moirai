//! Leitura e gravação que o app faz sobre o `moirai-core`; a mesma sequência do `moirai estimate`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hades_core::Env;
use hades_core::fsx::{is_file, read_text, realpath_missing_ok};
use hades_core::output::HadesError;
use hades_core::paths::hades_home;
use moirai_core::estimate::estimate_plan;
use moirai_core::history::{clock_ms, collect_history, window_days};
use moirai_core::limits::{LimitesDoPlano, limites_do_plano, limits_now};
use moirai_core::model::{HistoryUnit, Mode, Params, UnitEstimate};
use moirai_core::params::load_params;
use moirai_core::plan::{SectionInput, has_section, plan_units, render_section, write_section};
use moirai_core::report::costs_of;

/// Resultado de estimar um plano.
pub struct Estimativa {
    pub caminho: PathBuf,
    pub estimativas: Vec<UnitEstimate>,
    pub limites: LimitesDoPlano,
    pub base: usize,
}

/// Modo padrão pelo caminho: plano em `$HADES_HOME/plans/` é do skill (`src/commands/moirai.ts`).
pub fn modo_padrao(env: &Env, caminho: &Path) -> Mode {
    let plans = format!("{}/", hades_home(env).join("plans").to_string_lossy());
    if caminho.to_string_lossy().starts_with(&plans) {
        Mode::Skill
    } else {
        Mode::Projeto
    }
}

pub fn carregar_params(env: &Env, cwd: &Path) -> Result<Params, HadesError> {
    load_params(env, cwd)
}

pub fn carregar_historico(
    env: &Env,
    cwd: &Path,
    params: &Params,
) -> Result<Vec<HistoryUnit>, HadesError> {
    collect_history(env, cwd, params)
}

/// Estima o plano sem escrever nada.
pub fn estimar(
    env: &Env,
    cwd: &Path,
    plano: &Path,
    modo: Mode,
    params: &Params,
    historico: &[HistoryUnit],
) -> Result<Estimativa, HadesError> {
    if !is_file(plano) {
        return Err(HadesError::new(format!(
            "plano não encontrado: {}",
            plano.display()
        )));
    }
    let caminho = realpath_missing_ok(plano);
    let texto = read_text(&caminho).unwrap_or_default();
    let unidades = plan_units(&texto);
    if unidades.is_empty() {
        return Err(HadesError::new(format!(
            "{}: sem tabela em \"## Unidades\" (colunas Unidade, Papel e Escopo ou Specs); nada a estimar",
            caminho.display()
        )));
    }
    let estimativas = estimate_plan(&unidades, modo, historico, params, &HashMap::new());
    let medidas = limits_now(env, cwd)?;
    let limites = limites_do_plano(&costs_of(&estimativas), &medidas.calibracao, &medidas.atual);
    Ok(Estimativa {
        caminho,
        estimativas,
        limites,
        base: historico.len(),
    })
}

fn data_iso(ms: i64) -> String {
    let dias = ms.div_euclid(86_400_000);
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let ano = yoe + era * 400 + i64::from(mes <= 2);
    format!("{ano:04}-{mes:02}-{dia:02}")
}

/// Grava a seção "## Estimativa" no plano; devolve se substituiu uma anterior. Única escrita do app.
pub fn gravar_no_plano(
    env: &Env,
    cwd: &Path,
    e: &Estimativa,
    params: Params,
) -> Result<bool, HadesError> {
    let texto = read_text(&e.caminho).unwrap_or_default();
    let data = data_iso(clock_ms(env)?);
    let secao = render_section(&SectionInput {
        date: &data,
        estimates: &e.estimativas,
        history_size: e.base,
        window_days: window_days(env, cwd)?,
        params,
    });
    let substituiu = has_section(&texto);
    let novo = write_section(&texto, &secao);
    hades_core::atomic::write_file_atomic(&e.caminho, novo.as_bytes())
        .map_err(|err| HadesError::new(format!("{}: {err}", e.caminho.display())))?;
    Ok(substituiu)
}
