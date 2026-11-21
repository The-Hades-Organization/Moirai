use moirai_core::limits::{Atual, Faixa, Janela, LimiteDoPlano};
use moirai_core::model::{Band, Basis, Confidence, HistoryUnit, Mode, Spread, UnitEstimate};

use super::*;

fn spread(median: f64, p75: f64) -> Spread {
    Spread {
        n: 5,
        p25: median / 2.0,
        median,
        p75,
    }
}

fn est(
    nome: &str,
    custo: Option<(f64, f64)>,
    wall: Option<f64>,
    band: Option<Band>,
) -> UnitEstimate {
    UnitEstimate {
        unit: nome.into(),
        role: "impl".into(),
        mode: Mode::Projeto,
        declared_lines: None,
        band,
        basis: Basis::SemHistorico,
        sample: 0,
        confidence: Confidence::Nenhuma,
        cost_usd: custo.map(|(m, p)| spread(m, p)),
        wall_min: wall.map(|w| spread(w, w)),
        gate_min: None,
        tokens: None,
        note: String::new(),
    }
}

fn hist(nome: &str, role: &str, mode: Mode, band: Option<Band>, custo: Option<f64>) -> HistoryUnit {
    HistoryUnit {
        name: nome.into(),
        root: String::new(),
        mode,
        role: role.into(),
        state: "done".into(),
        created_at: String::new(),
        done_at: "2026-11-02T10:00:00.000Z".into(),
        cost_usd: custo,
        tokens: None,
        wall_s: Some(3600.0),
        gate_s: 120.0,
        gate_peak_mb: None,
        clears: 0.0,
        resumes: None,
        files: None,
        lines: None,
        band,
    }
}

fn limite(atual_pct: Option<f64>, plano_p75: Option<f64>, cabe: Option<bool>) -> LimiteDoPlano {
    LimiteDoPlano {
        por_unidade: Default::default(),
        plano: plano_p75.map(|p| Faixa {
            p25: p / 2.0,
            median: p / 1.5,
            p75: p,
        }),
        completo: true,
        atual: atual_pct.map(|pct| Atual {
            pct,
            resets_at: 0.0,
            at: String::new(),
        }),
        resta: None,
        cabe,
    }
}

#[test]
fn formata_numero_em_pt_br() {
    assert_eq!(numero(1234.5, 2), "1.234,50");
    assert_eq!(numero(6.1, 2), "6,10");
    assert_eq!(numero(-1234567.0, 0), "-1.234.567");
    assert_eq!(usd(41.2), "US$ 41,20");
    assert_eq!(usd_celula(None), "—");
}

#[test]
fn formata_duracao() {
    assert_eq!(duracao(580.0), "9h40");
    assert_eq!(duracao(45.0), "45min");
    assert_eq!(duracao(60.0), "1h00");
    assert_eq!(duracao_s(Some(5400.0)), "1h30");
    assert_eq!(duracao_s(None), "—");
}

#[test]
fn kpis_somam_so_o_que_tem_custo() {
    let e = [
        est("a", Some((6.0, 9.0)), Some(60.0), None),
        est("b", None, None, None),
        est("c", Some((4.0, 6.0)), Some(30.0), None),
    ];
    let k = kpis(&e, 38);
    assert_eq!(k.custo_p50, Some(10.0));
    assert_eq!(k.custo_p75, Some(15.0));
    assert_eq!(k.duracao_p50_min, Some(90.0));
    assert_eq!(k.base, 38);
    assert_eq!(kpis(&[est("x", None, None, None)], 0).custo_p50, None);
}

#[test]
fn ordem_alterna_direcao_e_reinicia_em_outra_coluna() {
    let a = Ordem::alternar(None, 3);
    assert_eq!(
        a,
        Ordem {
            coluna: 3,
            direcao: Direcao::Asc
        }
    );
    let b = Ordem::alternar(Some(a), 3);
    assert_eq!(b.direcao, Direcao::Desc);
    assert_eq!(Ordem::alternar(Some(b), 3).direcao, Direcao::Asc);
    assert_eq!(
        Ordem::alternar(Some(b), 1),
        Ordem {
            coluna: 1,
            direcao: Direcao::Asc
        }
    );
}

#[test]
fn ordenar_estimar_deixa_ausentes_no_fim_nas_duas_direcoes() {
    let e = [
        est("a", Some((6.0, 9.0)), None, None),
        est("b", None, None, None),
        est("c", Some((2.0, 3.0)), None, None),
    ];
    let mut l = linhas_estimar(&e);
    ordenar_estimar(
        &mut l,
        Some(Ordem {
            coluna: 3,
            direcao: Direcao::Asc,
        }),
    );
    assert_eq!(
        l.iter().map(|x| x.unidade.as_str()).collect::<Vec<_>>(),
        ["c", "a", "b"]
    );
    ordenar_estimar(
        &mut l,
        Some(Ordem {
            coluna: 3,
            direcao: Direcao::Desc,
        }),
    );
    assert_eq!(
        l.iter().map(|x| x.unidade.as_str()).collect::<Vec<_>>(),
        ["a", "c", "b"]
    );
}

#[test]
fn ordenar_por_faixa_usa_p_m_g() {
    let e = [
        est("g", None, None, Some(Band::Grande)),
        est("p", None, None, Some(Band::Pequena)),
        est("m", None, None, Some(Band::Media)),
    ];
    let mut l = linhas_estimar(&e);
    ordenar_estimar(
        &mut l,
        Some(Ordem {
            coluna: 2,
            direcao: Direcao::Asc,
        }),
    );
    assert_eq!(
        l.iter().map(|x| x.unidade.as_str()).collect::<Vec<_>>(),
        ["p", "m", "g"]
    );
}

#[test]
fn celulas_do_estimar_seguem_as_colunas() {
    let e = est(
        "hades-ui",
        Some((9.8, 16.2)),
        Some(160.0),
        Some(Band::Grande),
    );
    let c = celulas_estimar(&linhas_estimar(&[e])[0]);
    assert_eq!(c.len(), COLUNAS_ESTIMAR.len());
    assert_eq!(c[0], "hades-ui");
    assert_eq!(c[2], "G");
    assert_eq!(c[3], "9,80");
    assert_eq!(c[4], "16,20");
    assert_eq!(c[5], "2h40");
    assert_eq!(c[6], "—");
}

fn base() -> Vec<HistoryUnit> {
    vec![
        hist("u1", "impl", Mode::Projeto, Some(Band::Pequena), Some(3.0)),
        hist("u2", "impl", Mode::Skill, Some(Band::Media), Some(7.0)),
        hist("u3", "consolidate", Mode::Projeto, Some(Band::Grande), None),
        hist("u4", "review", Mode::Projeto, None, Some(1.0)),
    ]
}

#[test]
fn filtros_combinam_entre_dimensoes_e_unem_dentro_de_uma() {
    let h = base();
    assert_eq!(visiveis(&h, &Filtros::default(), None), [0, 1, 2, 3]);
    let mut f = Filtros::default();
    alternar_chip(&mut f, &Chip::Papel("impl".into()));
    alternar_chip(&mut f, &Chip::Papel("review".into()));
    assert_eq!(visiveis(&h, &f, None), [0, 1, 3]);
    alternar_chip(&mut f, &Chip::Modo(Mode::Projeto));
    assert_eq!(visiveis(&h, &f, None), [0, 3]);
    alternar_chip(&mut f, &Chip::Faixa(Band::Pequena));
    assert_eq!(visiveis(&h, &f, None), [0]);
    assert!(!f.vazio());
}

#[test]
fn chip_liga_e_desliga() {
    let mut f = Filtros::default();
    let c = Chip::Modo(Mode::Skill);
    alternar_chip(&mut f, &c);
    assert!(chip_ativo(&f, &c));
    alternar_chip(&mut f, &c);
    assert!(!chip_ativo(&f, &c));
    assert!(f.vazio());
}

#[test]
fn chips_listam_papeis_unicos_ordenados_depois_modos_e_faixas() {
    let chips = chips_do_historico(&base());
    let rotulos: Vec<String> = chips.iter().map(rotulo_do_chip).collect();
    assert_eq!(
        rotulos,
        [
            "consolidate",
            "impl",
            "review",
            "projeto",
            "skill",
            "faixa P",
            "faixa M",
            "faixa G"
        ]
    );
}

#[test]
fn historico_ordena_por_custo_com_ausente_no_fim() {
    let h = base();
    let f = Filtros::default();
    let asc = visiveis(
        &h,
        &f,
        Some(Ordem {
            coluna: 4,
            direcao: Direcao::Asc,
        }),
    );
    assert_eq!(asc, [3, 0, 1, 2]);
    let desc = visiveis(
        &h,
        &f,
        Some(Ordem {
            coluna: 4,
            direcao: Direcao::Desc,
        }),
    );
    assert_eq!(desc, [1, 0, 3, 2]);
}

#[test]
fn celulas_do_historico_seguem_as_colunas() {
    let c = celulas_historico(&base()[0]);
    assert_eq!(c.len(), COLUNAS_HISTORICO.len());
    assert_eq!(c[2], "projeto");
    assert_eq!(c[3], "P");
    assert_eq!(c[4], "3,00");
    assert_eq!(c[5], "1h00");
    assert_eq!(c[6], "2min");
    assert_eq!(c[7], "2026-11-02");
}

#[test]
fn modo_e_indice_do_seletor_sao_inversos() {
    for m in [Mode::Projeto, Mode::Skill] {
        assert_eq!(modo_de_indice(indice_do_modo(m)), m);
    }
}

#[test]
fn tons_da_janela_warn_na_projecao_e_bad_so_com_limite_estourado() {
    let neutro = limite(Some(38.0), Some(10.0), Some(true));
    assert_eq!(medidor(Janela::Five, &neutro).tom, Tone::Neutral);
    let projeta = limite(Some(71.0), Some(17.0), Some(false));
    let m = medidor(Janela::Seven, &projeta);
    assert_eq!(m.tom, Tone::Warn);
    assert_eq!(m.texto, "71% · o plano leva a ~88%");
    let estourou = limite(Some(100.0), Some(5.0), Some(false));
    assert_eq!(medidor(Janela::Five, &estourou).tom, Tone::Bad);
}

#[test]
fn medidor_sem_medida_fica_vazio_e_neutro() {
    let m = medidor(Janela::Five, &limite(None, None, None));
    assert_eq!(m.fracao, 0.0);
    assert_eq!(m.texto, "sem medida");
    assert_eq!(m.tom, Tone::Neutral);
}

#[test]
fn medidor_limita_a_fracao_a_um() {
    let m = medidor(Janela::Five, &limite(Some(140.0), None, None));
    assert_eq!(m.fracao, 1.0);
}

#[test]
fn teclas_trocam_de_tela() {
    assert_eq!(tela_da_tecla("1"), Some(Tela::Estimar));
    assert_eq!(tela_da_tecla("2"), Some(Tela::Historico));
    assert_eq!(tela_da_tecla("4"), Some(Tela::Janelas));
    assert_eq!(tela_da_tecla("0"), None);
    assert_eq!(tela_da_tecla("5"), None);
    assert_eq!(tela_da_tecla("a"), None);
    assert_eq!(Tela::de_rotulo("Histórico"), Some(Tela::Historico));
}

#[test]
fn colunas_numericas() {
    assert!(coluna_numerica("US$ p50"));
    assert!(coluna_numerica("Duração"));
    assert!(!coluna_numerica("Papel"));
}
