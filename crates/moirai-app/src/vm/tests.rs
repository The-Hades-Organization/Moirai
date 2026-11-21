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

fn painel_json() -> serde_json::Value {
    serde_json::json!({
        "v": 1, "warnPct": 100, "askPct": 150,
        "plans": [{
            "registry": "proj", "path": "/p.md", "approved": true,
            "units": [
                {"unit": "a", "estimate": {"costP75": 2.0}, "real": {"costUsd": 1.0, "state": "rodando"}, "pct": 50.0, "level": "ok"},
                {"unit": "b", "estimate": {"costP75": 1.0}, "real": {"costUsd": 2.0, "state": "rodando"}, "pct": 200.0, "level": "ask"},
                {"unit": "c", "estimate": null, "real": {"costUsd": 0.5, "state": null}, "pct": null, "level": null},
                {"unit": "d", "estimate": {"costP75": 3.0}, "real": null, "pct": null, "level": null},
                {"unit": "e", "estimate": {"costP75": 1.0}, "real": {"costUsd": 1.2, "state": "rodando"}, "pct": 120.0, "level": "warn"}
            ]
        }]
    })
}

#[test]
fn dado_o_json_do_painel_entao_so_unidade_com_custo_real_entra_da_mais_estourada() {
    let nomes: Vec<String> = linhas_teto(&painel_json())
        .into_iter()
        .map(|l| l.unidade)
        .collect();
    assert_eq!(nomes, ["b", "e", "a", "c"]);
}

#[test]
fn dado_unidade_sem_estimativa_entao_celulas_mostram_travessao_e_sem_estimativa() {
    let linhas = linhas_teto(&painel_json());
    let c = linhas.iter().find(|l| l.unidade == "c").unwrap();
    assert_eq!(
        celulas_teto(c),
        ["c", "proj", "—", "US$ 0,50", "—", "—", "sem estimativa"]
    );
}

#[test]
fn dado_um_nivel_entao_o_tom_e_bad_so_ao_estourar_a_pergunta() {
    use moirai_core::budget::BudgetLevel::{Ask, Ok, Warn};
    assert_eq!(tom_do_nivel(Some(Ask)), Tone::Bad);
    assert_eq!(tom_do_nivel(Some(Warn)), Tone::Warn);
    assert_eq!(tom_do_nivel(Some(Ok)), Tone::Ok);
    assert_eq!(tom_do_nivel(None), Tone::Neutral);
}

#[test]
fn dado_json_sem_planos_entao_a_lista_fica_vazia_e_limiares_vem_do_painel() {
    assert!(linhas_teto(&serde_json::json!({})).is_empty());
    assert_eq!(limiares(&painel_json()), Some((100.0, 150.0)));
    assert_eq!(limiares(&serde_json::json!({})), None);
}

#[test]
fn dado_o_instante_de_renovacao_entao_o_texto_usa_a_maior_unidade() {
    let agora = 1_000_000.0 * 1000.0;
    assert_eq!(renovacao(1_000_000.0 - 5.0, agora), "renovando agora");
    assert_eq!(
        renovacao(1_000_000.0 + 25.0 * 60.0, agora),
        "renova em 25min"
    );
    assert_eq!(
        renovacao(1_000_000.0 + 135.0 * 60.0, agora),
        "renova em 2h15"
    );
    assert_eq!(
        renovacao(1_000_000.0 + 3.0 * 86_400.0 + 5.0 * 3600.0, agora),
        "renova em 3d 5h"
    );
}

#[test]
fn dado_janela_sem_medida_entao_o_cartao_diz_sem_medida() {
    let limite = LimiteDoPlano {
        por_unidade: Default::default(),
        plano: None,
        completo: true,
        atual: None,
        resta: None,
        cabe: None,
    };
    let c = cartao_da_janela(Janela::Five, &limite, 0.0);
    assert_eq!(c.renovacao, "sem medida");
    assert_eq!(c.plano, "plano aberto sem estimativa calibrada");
    assert_eq!(c.medidor.texto, "sem medida");
}

#[test]
fn dado_plano_parcial_entao_o_cartao_marca_parcial_e_mostra_a_faixa() {
    let limite = LimiteDoPlano {
        por_unidade: Default::default(),
        plano: Some(Faixa {
            p25: 4.0,
            median: 6.0,
            p75: 9.0,
        }),
        completo: false,
        atual: Some(Atual {
            pct: 40.0,
            resets_at: 1_000_600.0,
            at: "x".into(),
        }),
        resta: None,
        cabe: Some(true),
    };
    let c = cartao_da_janela(Janela::Seven, &limite, 1_000_000.0 * 1000.0);
    assert_eq!(c.plano, "plano aberto: ~4% a ~9% (parcial)");
    assert_eq!(c.renovacao, "renova em 10min");
}

#[test]
fn dado_duas_janelas_entao_a_linha_do_status_junta_com_ponto_medio() {
    let a = Atual {
        pct: 42.0,
        resets_at: 1.0,
        at: "x".into(),
    };
    let linha = linha_das_janelas(&[(Janela::Five, Some(&a)), (Janela::Seven, None)]);
    assert_eq!(
        linha,
        format!(
            "{} 42% · {} sem medida",
            Janela::Five.rotulo(),
            Janela::Seven.rotulo()
        )
    );
}

#[test]
fn dado_a_mesma_assinatura_entao_a_gravacao_nao_dispara_releitura() {
    let a = Assinatura {
        bytes: 10,
        mtime_ms: 5,
    };
    assert!(!mudou(Some(a), Some(a)));
    assert!(mudou(
        Some(a),
        Some(Assinatura {
            bytes: 11,
            mtime_ms: 5
        })
    ));
    assert!(mudou(Some(a), None));
}

#[test]
fn dado_a_coluna_de_percentual_entao_e_numerica() {
    assert!(coluna_numerica("% do p75"));
}

#[test]
fn dado_tarefa_em_voo_entao_a_renovacao_pula() {
    let mut t = Tarefa::default();
    let g = t.iniciar().unwrap();
    assert!(t.carregando());
    assert_eq!(t.iniciar(), None);
    assert!(t.concluir(g));
    assert!(!t.carregando());
    assert!(t.iniciar().is_some());
}

#[test]
fn dado_resultado_de_geracao_velha_entao_e_descartado_sem_liberar_a_nova() {
    let mut t = Tarefa::default();
    let velha = t.iniciar().unwrap();
    t.invalidar();
    let nova = t.iniciar().unwrap();
    assert_ne!(velha, nova);
    assert!(!t.concluir(velha));
    assert!(t.carregando());
    assert!(t.concluir(nova));
    assert!(!t.carregando());
}

#[test]
fn dado_calculo_em_outra_thread_entao_o_resultado_chega_pelo_controle_da_tarefa() {
    let dir = std::env::temp_dir().join(format!("moirai-vm-{}", std::process::id()));
    let env = hades_core::Env::empty().with("HOME", dir.to_str().unwrap());
    let painel = std::sync::Arc::new(std::sync::Mutex::new(crate::service::novo_painel(
        &env, &dir,
    )));
    let mut t = Tarefa::default();
    let g = t.iniciar().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let p = std::sync::Arc::clone(&painel);
    std::thread::spawn(move || {
        let plano = p.lock().unwrap().plan();
        tx.send((g, linhas_teto(&plano), limiares(&plano))).unwrap();
    });
    let (geracao, linhas, _) = rx.recv().unwrap();
    assert!(t.concluir(geracao));
    assert!(linhas.is_empty());
}

fn dicas(n: u64) -> std::collections::HashMap<String, f64> {
    [("u1".to_owned(), n as f64)].into()
}

#[test]
fn dado_profunda_com_sucesso_em_outra_thread_entao_as_dicas_sao_aplicadas() {
    let mut p = Profunda::default();
    let g = p.iniciar().unwrap();
    assert!(p.estimando());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send((g, Ok(dicas(120)))).unwrap());
    let (g, r) = rx.recv().unwrap();
    assert_eq!(p.concluir(g, r), Profundo::Aplicado);
    assert!(!p.estimando());
    assert_eq!(p.dicas().get("u1"), Some(&120.0));
}

#[test]
fn dado_profunda_em_andamento_entao_o_segundo_clique_e_bloqueado() {
    let mut p = Profunda::default();
    let g = p.iniciar().unwrap();
    assert_eq!(p.iniciar(), None);
    assert_eq!(p.concluir(g, Ok(dicas(1))), Profundo::Aplicado);
    assert!(p.iniciar().is_some());
}

#[test]
fn dado_profunda_com_erro_entao_devolve_a_mensagem_e_libera_o_botao() {
    let mut p = Profunda::default();
    let g = p.iniciar().unwrap();
    let r = p.concluir(
        g,
        Err("moirai --deep: o comando `hades` não está no PATH".into()),
    );
    assert_eq!(
        r,
        Profundo::Falhou("moirai --deep: o comando `hades` não está no PATH".into())
    );
    assert!(!p.estimando());
    assert!(p.dicas().is_empty());
}

#[test]
fn dado_plano_que_mudou_entao_o_resultado_velho_e_descartado() {
    let mut p = Profunda::default();
    let velha = p.iniciar().unwrap();
    p.esquecer();
    assert!(!p.estimando());
    assert_eq!(p.concluir(velha, Ok(dicas(9))), Profundo::Descartado);
    assert!(p.dicas().is_empty());
}

#[test]
fn dado_plano_que_mudou_entao_as_dicas_antigas_sao_esquecidas() {
    let mut p = Profunda::default();
    let g = p.iniciar().unwrap();
    p.concluir(g, Ok(dicas(5)));
    p.esquecer();
    assert!(p.dicas().is_empty());
}
