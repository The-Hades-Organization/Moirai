use super::*;
use std::fs;
use tempfile::TempDir;

const KIT: &str = "\
HADES_MOIRAI_MIN_SAMPLE\t5\tint:1:100\tOrquestração\tAmostra mínima\tnext\tUnidades parecidas.\n\
HADES_MOIRAI_WARN_PCT\t100\tint:1:10000\tOrquestração\tAviso de custo\tnext\tAviso.\n\
HADES_OUTRA\t1\tbool\tG\tOutra\tnext\tNão é do Moirai.\n";

struct Sandbox {
    _dir: TempDir,
    env: Env,
    store: ConfigStore,
    cwd: std::path::PathBuf,
}

fn sandbox() -> Sandbox {
    let dir = TempDir::new().unwrap();
    let skill = dir.path().join("skill");
    let kit = skill.join("kit");
    let dados = dir.path().join("dados");
    fs::create_dir_all(&kit).unwrap();
    fs::create_dir_all(&dados).unwrap();
    fs::write(kit.join("config.tsv"), KIT).unwrap();
    let env = Env::empty()
        .with("HOME", dir.path().to_string_lossy())
        .with("HADES_SKILL_DIR", skill.to_string_lossy())
        .with("HADES_DATA", dados.to_string_lossy());
    Sandbox {
        cwd: dir.path().to_path_buf(),
        store: ConfigStore { dir: dados, kit },
        env,
        _dir: dir,
    }
}

fn textos(linhas: &[Linha]) -> Vec<String> {
    linhas.iter().map(|l| l.valor.clone()).collect()
}

#[test]
fn mostra_so_as_chaves_do_moirai_com_padrao_e_faixa() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let chaves: Vec<_> = linhas.iter().map(|l| l.chave.as_str()).collect();
    assert_eq!(chaves, ["HADES_MOIRAI_MIN_SAMPLE", "HADES_MOIRAI_WARN_PCT"]);
    assert_eq!(linhas[0].faixa(), Some((1, 100)));
    assert_eq!(linhas[0].valor, "5");
    assert_eq!(linhas[0].origem, Origem::Padrao);
}

#[test]
fn gravar_pela_pagina_e_ler_pelo_config_value_do_cli() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = " 12 ".into();
    assert_eq!(pendentes(&linhas, &t), 1);
    assert_eq!(salvar(&fonte, &linhas, &t), Ok(1));
    assert_eq!(
        config::config_value(&s.env, &s.cwd, "HADES_MOIRAI_MIN_SAMPLE", ""),
        "12"
    );
    assert_eq!(
        config::config_value(&s.env, &s.cwd, "HADES_MOIRAI_WARN_PCT", ""),
        "100"
    );
    let depois = fonte.carregar().unwrap();
    assert_eq!(depois[0].origem, Origem::Arquivo);
    assert_eq!(pendentes(&depois, &textos(&depois)), 0);
}

#[test]
fn grava_no_config_env_do_estado_e_nao_na_skill() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[1] = "250".into();
    salvar(&fonte, &linhas, &t).unwrap();
    let env_txt = fs::read_to_string(s.store.dir.join("config.env")).unwrap();
    assert!(env_txt.contains("HADES_MOIRAI_WARN_PCT=250\n"));
    assert_eq!(
        fs::read_to_string(s.store.kit.join("config.tsv")).unwrap(),
        KIT
    );
}

#[test]
fn valor_fora_da_faixa_ou_nao_inteiro_e_recusado_sem_gravar() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    for ruim in ["0", "101", "-3", "abc", "", "1.5", "+7", "1e2"] {
        let mut t = textos(&linhas);
        t[0] = ruim.into();
        assert!(erros(&linhas, &t)[0].is_some(), "{ruim}");
        assert!(salvar(&fonte, &linhas, &t).is_err(), "{ruim}");
    }
    assert!(!s.store.dir.join("config.env").exists());
    assert_eq!(
        erros(&linhas, &{
            let mut t = textos(&linhas);
            t[0] = "101".into();
            t
        })[0]
            .as_deref(),
        Some("Informe um número inteiro entre 1 e 100")
    );
}

#[test]
fn zeros_a_esquerda_normalizam_como_o_ts() {
    let s = sandbox();
    let linhas = ConfigLocal::new(&s.env, &s.cwd).carregar().unwrap();
    assert_eq!(validar(&linhas[0], " 007 "), Ok("7".to_owned()));
}

#[test]
fn restaurar_padrao_tira_a_chave_do_arquivo() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = "40".into();
    salvar(&fonte, &linhas, &t).unwrap();

    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = linhas[0].padrao.clone();
    assert_eq!(pendentes(&linhas, &t), 1);
    assert_eq!(salvar(&fonte, &linhas, &t), Ok(1));
    assert_eq!(
        config::config_value(&s.env, &s.cwd, "HADES_MOIRAI_MIN_SAMPLE", ""),
        "5"
    );
    let env_txt = fs::read_to_string(s.store.dir.join("config.env")).unwrap();
    assert!(!env_txt.contains("HADES_MOIRAI_MIN_SAMPLE"));
    assert_eq!(fonte.carregar().unwrap()[0].origem, Origem::Padrao);
}

#[test]
fn valor_do_ambiente_vence_o_arquivo_e_fica_so_leitura() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = "40".into();
    salvar(&fonte, &linhas, &t).unwrap();

    let env = s.env.with("HADES_MOIRAI_MIN_SAMPLE", "9");
    let fonte = ConfigLocal::new(&env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    assert_eq!(linhas[0].valor, "9");
    assert_eq!(linhas[0].origem, Origem::Ambiente);
    assert!(linhas[0].somente_leitura());
    assert_eq!(
        config::config_value(&env, &s.cwd, "HADES_MOIRAI_MIN_SAMPLE", ""),
        "9"
    );
    let mut t = textos(&linhas);
    t[0] = "50".into();
    assert_eq!(pendentes(&linhas, &t), 0);
    assert_eq!(erros(&linhas, &t)[0], None);
    assert_eq!(salvar(&fonte, &linhas, &t), Ok(0));
    let env_txt = fs::read_to_string(s.store.dir.join("config.env")).unwrap();
    assert!(env_txt.contains("HADES_MOIRAI_MIN_SAMPLE=40\n"));
}

#[test]
fn uma_linha_invalida_impede_gravar_as_outras() {
    let s = sandbox();
    let fonte = ConfigLocal::new(&s.env, &s.cwd);
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = "20".into();
    t[1] = "0".into();
    let erro = salvar(&fonte, &linhas, &t).unwrap_err();
    assert!(erro.starts_with("Aviso de custo:"), "{erro}");
    assert!(!s.store.dir.join("config.env").exists());
}

#[test]
fn sem_instalacao_nao_grava() {
    let dir = TempDir::new().unwrap();
    let skill = dir.path().join("skill");
    fs::create_dir_all(skill.join("kit")).unwrap();
    fs::write(skill.join("kit/config.tsv"), KIT).unwrap();
    let env = Env::empty()
        .with("HOME", dir.path().to_string_lossy())
        .with("HADES_SKILL_DIR", skill.to_string_lossy());
    let fonte = ConfigLocal::new(&env, dir.path());
    assert!(!fonte.gravavel());
    let linhas = fonte.carregar().unwrap();
    let mut t = textos(&linhas);
    t[0] = "20".into();
    assert!(salvar(&fonte, &linhas, &t).is_err());
    assert!(!skill.join("kit/config.env").exists());
}

#[test]
fn faixa_do_tipo_so_para_int() {
    assert_eq!(faixa_do_tipo("int:0:3650"), Some((0, 3650)));
    assert_eq!(faixa_do_tipo("bool"), None);
}
