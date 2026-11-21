//! Configuração do Moirai (`src/commands/moirai.ts:39-53`).

use std::path::Path;

use hades_core::config::config_value;
use hades_core::env::Env;
use hades_core::output::HadesError;

use crate::jsnum::js_number;
use crate::model::Params;

/// `intKey` (`src/commands/moirai.ts:39`): inteiro maior ou igual a zero, senão recusa com código de uso.
fn int_key(env: &Env, cwd: &Path, key: &str, fallback: u32) -> Result<f64, HadesError> {
    let raw = config_value(env, cwd, key, &fallback.to_string());
    match js_number(&raw) {
        Some(value) if value.is_finite() && value.fract() == 0.0 && value >= 0.0 => Ok(value),
        _ => Err(HadesError::new(format!(
            "{key}={raw}: esperava um inteiro maior ou igual a zero"
        ))),
    }
}

/// `loadParams` (`src/commands/moirai.ts:46`).
pub fn load_params(env: &Env, cwd: &Path) -> Result<Params, HadesError> {
    Ok(Params {
        min_sample: int_key(env, cwd, "HADES_MOIRAI_MIN_SAMPLE", 5)?.max(1.0),
        high_factor: int_key(env, cwd, "HADES_MOIRAI_HIGH_FACTOR", 3)?.max(1.0),
        small_lines: int_key(env, cwd, "HADES_MOIRAI_SMALL_LINES", 150)?,
        large_lines: int_key(env, cwd, "HADES_MOIRAI_LARGE_LINES", 600)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn dado_nada_configurado_entao_valem_os_padroes() {
        let dir = cwd();
        let env = Env::empty().with("HOME", dir.path().to_str().unwrap());
        let p = load_params(&env, dir.path()).unwrap();
        assert_eq!(
            p,
            Params {
                min_sample: 5.0,
                high_factor: 3.0,
                small_lines: 150.0,
                large_lines: 600.0
            }
        );
    }

    #[test]
    fn dado_minimo_zero_entao_sobe_para_um_e_limites_aceitam_zero() {
        let dir = cwd();
        let env = Env::empty()
            .with("HOME", dir.path().to_str().unwrap())
            .with("HADES_MOIRAI_MIN_SAMPLE", "0")
            .with("HADES_MOIRAI_HIGH_FACTOR", "0")
            .with("HADES_MOIRAI_SMALL_LINES", "0");
        let p = load_params(&env, dir.path()).unwrap();
        assert_eq!(
            (p.min_sample, p.high_factor, p.small_lines),
            (1.0, 1.0, 0.0)
        );
    }

    #[test]
    fn dado_valor_negativo_ou_nao_inteiro_entao_recusa_com_codigo_2() {
        let dir = cwd();
        for bad in ["-1", "1.5", "abc"] {
            let env = Env::empty()
                .with("HOME", dir.path().to_str().unwrap())
                .with("HADES_MOIRAI_SMALL_LINES", bad);
            let e = load_params(&env, dir.path()).unwrap_err();
            assert_eq!(e.code, 2);
            assert_eq!(
                e.message,
                format!(
                    "HADES_MOIRAI_SMALL_LINES={bad}: esperava um inteiro maior ou igual a zero"
                )
            );
        }
    }
}
