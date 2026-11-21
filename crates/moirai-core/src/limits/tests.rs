//! Casos de `loop/test/ts/limites.test.ts` (regra) e da borda de leitura com HOME falso.

use std::fs;

use serde_json::json;

use super::*;

const OPCOES: Opcoes = Opcoes {
    min_delta_pct: 1.0,
    min_intervals: 5.0,
    cache_read_weight: 0.1,
};
const T0: i64 = 1_793_527_200; // 2026-11-01T10:00:00Z
const MIN: i64 = 60;
const RENOVA: f64 = (T0 + 5 * 3600) as f64;

fn at(s: i64) -> String {
    iso_ms(s * 1000)
}

fn sample(s: i64, five: f64, resets_at: f64, seven: Option<f64>) -> Amostra {
    Amostra {
        at: at(s),
        five: Some(Leitura {
            pct: five,
            resets_at,
        }),
        seven: seven.map(|pct| Leitura {
            pct,
            resets_at: (T0 + 7 * 86_400) as f64,
        }),
        version: None,
    }
}

fn s5(s: i64, five: f64) -> Amostra {
    sample(s, five, RENOVA, None)
}

fn req(s: i64, model: &str, milhoes: f64, usd: f64) -> Req {
    Req {
        epoch_s: s,
        model: model.to_owned(),
        tok: [milhoes * 1e6, 0.0, 0.0, 0.0],
        cost: usd,
    }
}

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() < eps, "{a} != {b}");
}

fn atual(pct: f64) -> Atual {
    Atual {
        pct,
        resets_at: RENOVA,
        at: at(T0),
    }
}

mod status_line {
    use super::*;

    fn doc(rate_limits: serde_json::Value) -> String {
        json!({"version": "2.1.317", "rate_limits": rate_limits}).to_string()
    }

    #[test]
    fn dado_as_duas_janelas_entao_a_amostra_leva_pct_e_renovacao_de_cada_uma() {
        let a = amostra_do_status_line(
            &doc(json!({
                "five_hour": {"used_percentage": 23.5, "resets_at": 1738425600},
                "seven_day": {"used_percentage": 41.2, "resets_at": 1738857600}})),
            T0 * 1000,
        )
        .unwrap();
        assert_eq!(a.at, "2026-11-01T10:00:00.000Z");
        assert_eq!(
            a.five,
            Some(Leitura {
                pct: 23.5,
                resets_at: 1738425600.0
            })
        );
        assert_eq!(
            a.seven,
            Some(Leitura {
                pct: 41.2,
                resets_at: 1738857600.0
            })
        );
        assert_eq!(a.version.as_deref(), Some("2.1.317"));
    }

    #[test]
    fn dado_uma_janela_so_entao_a_outra_fica_ausente() {
        let a = amostra_do_status_line(
            &doc(json!({"seven_day": {"used_percentage": 3, "resets_at": 5}})),
            T0 * 1000,
        )
        .unwrap();
        assert_eq!(a.five, None);
        assert_eq!(
            a.seven,
            Some(Leitura {
                pct: 3.0,
                resets_at: 5.0
            })
        );
    }

    #[test]
    fn dado_entrada_sem_leitura_valida_entao_nao_ha_amostra() {
        let casos = [
            json!({"model": "x"}).to_string(),
            doc(json!({})),
            doc(json!({"five_hour": {"used_percentage": "9", "resets_at": 5}})),
            doc(json!({"five_hour": {"used_percentage": -1, "resets_at": 5}})),
            doc(json!({"five_hour": {"used_percentage": 9}})),
            "{ não é json".to_owned(),
            String::new(),
        ];
        for texto in casos {
            assert_eq!(amostra_do_status_line(&texto, T0 * 1000), None, "{texto}");
        }
    }

    #[test]
    fn dado_arquivo_com_linha_corrompida_entao_so_ela_e_ignorada_e_as_demais_vem_em_ordem_de_horario()
     {
        let grava = |a: &Amostra| {
            json!({"at": a.at, "five": {"pct": a.five.unwrap().pct, "resetsAt": a.five.unwrap().resets_at}})
                .to_string()
        };
        let linhas = [
            grava(&s5(T0 + 60, 5.0)),
            "lixo".to_owned(),
            json!({"at": "x"}).to_string(),
            grava(&s5(T0, 4.0)),
            String::new(),
        ]
        .join("\n");
        let pcts: Vec<f64> = amostras_de(&linhas)
            .iter()
            .map(|a| a.five.unwrap().pct)
            .collect();
        assert_eq!(pcts, [4.0, 5.0]);
    }

    #[test]
    fn dado_linha_gravada_sem_janela_valida_ou_horario_invalido_entao_descarta() {
        let linhas = [
            json!({"at": "2026-11-01T10:00:00.000Z"}).to_string(),
            json!({"at": "2026-11-01T10:00:00.000Z", "five": {"pct": -1, "resetsAt": 5}}).to_string(),
            json!({"at": 5, "five": {"pct": 1, "resetsAt": 5}}).to_string(),
            json!({"at": "2026-11-01T10:00:00.000Z", "seven": {"pct": 1, "resetsAt": 5}, "version": "9"})
                .to_string(),
        ]
        .join("\n");
        let lidas = amostras_de(&linhas);
        assert_eq!(lidas.len(), 1);
        assert_eq!(lidas[0].version.as_deref(), Some("9"));
        assert_eq!(lidas[0].five, None);
    }
}

mod quando_gravar {
    use super::*;

    #[test]
    fn dado_a_mesma_leitura_entao_nao_grava_mesmo_passado_o_intervalo() {
        let a = s5(T0, 10.0);
        assert!(!vale(&s5(T0 + 3600, 10.0), Some(&a), 60.0));
    }

    #[test]
    fn dado_leitura_nova_antes_do_intervalo_entao_nao_grava_no_intervalo_grava() {
        let a = s5(T0, 10.0);
        assert!(!vale(&s5(T0 + 59, 11.0), Some(&a), 60.0));
        assert!(vale(&s5(T0 + 60, 11.0), Some(&a), 60.0));
    }

    #[test]
    fn dado_nenhuma_amostra_anterior_entao_grava() {
        assert!(vale(&s5(T0, 10.0), None, 60.0));
    }

    #[test]
    fn dado_retencao_entao_descarta_o_mais_velho_que_o_corte() {
        let amostras = [s5(T0 - 3 * 86_400, 1.0), s5(T0 - 86_400, 2.0)];
        let now = (T0 * 1000) as f64;
        assert_eq!(dentro_da_retencao(&amostras, now, 2.0).len(), 1);
        assert_eq!(dentro_da_retencao(&amostras, now, 3.0).len(), 2);
    }
}

mod medida_atual {
    use super::*;

    #[test]
    fn dado_janela_ja_renovada_entao_nao_conta_como_medida_a_ainda_aberta_sim() {
        let amostras = [Amostra {
            at: at(T0),
            five: Some(Leitura {
                pct: 90.0,
                resets_at: (T0 - 1) as f64,
            }),
            seven: Some(Leitura {
                pct: 40.0,
                resets_at: (T0 + 86_400) as f64,
            }),
            version: None,
        }];
        let a = atuais(&amostras, (T0 * 1000) as f64);
        assert_eq!(a.five, None);
        assert_eq!(a.seven.unwrap().pct, 40.0);
    }

    #[test]
    fn dado_duas_amostras_entao_vale_a_ultima() {
        let a = atuais(
            &[s5(T0, 10.0), s5(T0 + 60, 12.0)],
            ((T0 + 120) * 1000) as f64,
        );
        assert_eq!(a.five.unwrap().pct, 12.0);
    }

    #[test]
    fn dado_renovacao_igual_ao_agora_entao_a_janela_ja_venceu() {
        let a = atuais(&[sample(T0, 5.0, T0 as f64, None)], (T0 * 1000) as f64);
        assert_eq!(a.five, None);
    }
}

mod calibracao {
    use super::*;

    /// 8 passos de 10 min; a cada passo, `milhoes` de tokens e `taxa` % por milhão.
    fn serie(taxa: f64, milhoes: f64, model: &str) -> (Vec<Amostra>, Vec<Req>) {
        let mut amostras = vec![s5(T0, 0.0)];
        let mut reqs = Vec::new();
        for i in 1..=8 {
            reqs.push(req(T0 + i * 10 * MIN - 5 * MIN, model, milhoes, milhoes));
            amostras.push(s5(T0 + i * 10 * MIN, i as f64 * taxa * milhoes));
        }
        (amostras, reqs)
    }

    #[test]
    fn dado_2_por_cento_por_milhao_em_todos_os_passos_entao_a_taxa_do_modelo_e_a_por_usd_saem_2() {
        let (amostras, reqs) = serie(2.0, 1.0, "opus-5-5");
        let cal = calibra(&amostras, &reqs, &OPCOES);
        close(cal.five.por_modelo["opus-5-5"], 2.0, 1e-6);
        close(cal.five.por_usd.unwrap(), 2.0, 1e-6);
        assert_eq!(cal.five.intervalos, 8);
    }

    #[test]
    fn dado_dois_modelos_em_misturas_diferentes_entao_cada_taxa_e_recuperada() {
        let mut amostras = vec![s5(T0, 0.0)];
        let mut reqs = Vec::new();
        let mut pct = 0.0;
        let mix = [
            (1.0, 0.0),
            (0.0, 2.0),
            (1.0, 1.0),
            (2.0, 1.0),
            (1.0, 3.0),
            (3.0, 0.0),
            (0.0, 1.0),
            (2.0, 2.0),
        ];
        for (i, (opus, sonnet)) in mix.into_iter().enumerate() {
            let t = T0 + (i as i64 + 1) * 10 * MIN;
            if opus != 0.0 {
                reqs.push(req(t - 60, "opus-5-5", opus, opus * 4.0));
            }
            if sonnet != 0.0 {
                reqs.push(req(t - 30, "sonnet-5-5", sonnet, sonnet * 2.0));
            }
            pct += opus * 3.0 + sonnet;
            amostras.push(s5(t, pct));
        }
        let cal = calibra(&amostras, &reqs, &OPCOES);
        close(cal.five.por_modelo["opus-5-5"], 3.0, 1e-3);
        close(cal.five.por_modelo["sonnet-5-5"], 1.0, 1e-3);
    }

    #[test]
    fn dado_menos_intervalos_que_o_minimo_entao_sem_calibracao_nada_estimado() {
        let (amostras, reqs) = serie(2.0, 1.0, "opus-5-5");
        let cal = calibra(&amostras[..5], &reqs, &OPCOES);
        assert!(cal.five.por_modelo.is_empty());
        assert_eq!(cal.five.por_usd, None);
        assert_eq!(cal.five.intervalos, 4);
        let tok: IndexMap<String, Tok> = [("opus-5-5".to_owned(), [1e6, 0.0, 0.0, 0.0])].into();
        assert_eq!(
            texto_pct(&pct_de_tokens(&cal.five, Some(&tok), 0.1)),
            "sem calib."
        );
    }

    #[test]
    fn dado_modelo_com_poucos_intervalos_entao_so_ele_fica_sem_taxa_e_o_pct_sai_parcial() {
        let (amostras, mut reqs) = serie(2.0, 1.0, "opus-5-5");
        reqs.push(req(T0 + 25 * MIN, "haiku-5-5", 1.0, 1.0));
        let cal = calibra(&amostras, &reqs, &OPCOES);
        assert!(!cal.five.por_modelo.contains_key("haiku-5-5"));
        let tok: IndexMap<String, Tok> = [
            ("opus-5-5".to_owned(), [1e6, 0.0, 0.0, 0.0]),
            ("haiku-5-5".to_owned(), [1e6, 0.0, 0.0, 0.0]),
        ]
        .into();
        let est = pct_de_tokens(&cal.five, Some(&tok), 0.1);
        close(est.pct.unwrap(), 2.0, 1e-5);
        assert!(est.parcial);
        assert_eq!(texto_pct(&est), "~2.0%+");
    }

    #[test]
    fn dado_janela_que_renovou_no_meio_entao_o_intervalo_que_a_cruza_e_descartado() {
        let nova = RENOVA + 5.0 * 3600.0;
        let amostras = [
            s5(T0, 0.0),
            s5(T0 + 600, 2.0),
            s5(T0 + 1200, 4.0),
            sample(T0 + 1800, 1.0, nova, None),
            sample(T0 + 2400, 3.0, nova, None),
        ];
        let reqs = [
            req(T0 + 300, "opus-5-5", 1.0, 1.0),
            req(T0 + 900, "opus-5-5", 1.0, 1.0),
            req(T0 + 1500, "opus-5-5", 1.0, 1.0),
            req(T0 + 2100, "opus-5-5", 1.0, 1.0),
        ];
        let deltas: Vec<f64> = intervalos(&amostras, Janela::Five, &reqs, &OPCOES)
            .iter()
            .map(|i| i.d_pct)
            .collect();
        assert_eq!(deltas, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn dado_pct_que_nao_passa_da_variacao_minima_entao_o_intervalo_acumula_ate_passar() {
        let amostras = [s5(T0, 0.0), s5(T0 + 600, 0.4), s5(T0 + 1200, 1.2)];
        let reqs = [
            req(T0 + 300, "opus-5-5", 1.0, 1.0),
            req(T0 + 900, "opus-5-5", 1.0, 1.0),
        ];
        let itens = intervalos(&amostras, Janela::Five, &reqs, &OPCOES);
        assert_eq!(itens.len(), 1);
        close(itens[0].d_pct, 1.2, 1e-9);
        close(itens[0].mtok["opus-5-5"], 2.0, 1e-9);
    }

    #[test]
    fn dado_a_janela_semanal_entao_ela_calibra_a_parte_da_de_5_h() {
        let amostras: Vec<Amostra> = (0..9)
            .map(|i| sample(T0 + i * 600, (i * 4) as f64, RENOVA, Some(i as f64)))
            .collect();
        let reqs: Vec<Req> = (0..8)
            .map(|i| req(T0 + i * 600 + 300, "opus-5-5", 2.0, 2.0))
            .collect();
        let cal = calibra(&amostras, &reqs, &OPCOES);
        close(cal.five.por_modelo["opus-5-5"], 2.0, 1e-6);
        close(cal.seven.por_modelo["opus-5-5"], 0.5, 1e-6);
    }

    #[test]
    fn dado_cache_de_leitura_entao_pesa_a_fracao_configurada_de_um_token_novo() {
        close(mtok(&[0.0, 0.0, 0.0, 1e6], 0.1), 0.1, 1e-9);
        close(mtok(&[1e6, 1e6, 1e6, 0.0], 0.1), 3.0, 1e-9);
    }

    #[test]
    fn dado_nenhuma_amostra_entao_tudo_sem_calibracao() {
        let cal = calibra(&[], &[], &OPCOES);
        let vazio = Taxas {
            por_modelo: IndexMap::new(),
            por_usd: None,
            intervalos: 0,
        };
        assert_eq!(cal.five, vazio);
        assert_eq!(cal.seven, vazio);
    }

    #[test]
    fn dado_delta_negativo_entao_reinicia_a_ancora_sem_gerar_intervalo() {
        let amostras = [s5(T0, 10.0), s5(T0 + 600, 5.0), s5(T0 + 1200, 8.0)];
        let reqs = [
            req(T0 + 300, "opus-5-5", 1.0, 1.0),
            req(T0 + 900, "opus-5-5", 1.0, 1.0),
        ];
        let itens = intervalos(&amostras, Janela::Five, &reqs, &OPCOES);
        assert_eq!(itens.len(), 1);
        assert_eq!(itens[0].d_pct, 3.0);
        close(itens[0].mtok["opus-5-5"], 1.0, 1e-9);
    }
}

mod cabe_na_janela_e_plano {
    use super::*;

    fn cabe(resta: Option<f64>, cabe: Option<bool>) -> Cabe {
        Cabe { resta, cabe }
    }

    #[test]
    fn dado_p75_abaixo_do_que_resta_entao_cabe_igual_ainda_cabe_acima_nao() {
        let a = atual(70.0);
        assert_eq!(
            cabe_na_janela(Some(&a), Some(29.0)),
            cabe(Some(30.0), Some(true))
        );
        assert_eq!(
            cabe_na_janela(Some(&a), Some(30.0)),
            cabe(Some(30.0), Some(true))
        );
        assert_eq!(
            cabe_na_janela(Some(&a), Some(30.1)),
            cabe(Some(30.0), Some(false))
        );
    }

    #[test]
    fn dado_janela_ja_estourada_entao_resta_0_e_nada_cabe() {
        assert_eq!(
            cabe_na_janela(Some(&atual(104.0)), Some(1.0)),
            cabe(Some(0.0), Some(false))
        );
    }

    #[test]
    fn dado_sem_medida_atual_ou_sem_p75_calibrado_entao_nao_da_para_dizer() {
        assert_eq!(cabe_na_janela(None, Some(5.0)), cabe(None, None));
        assert_eq!(
            cabe_na_janela(Some(&atual(70.0)), None),
            cabe(Some(30.0), None)
        );
    }

    fn cal() -> Calibracao {
        Calibracao {
            five: Taxas {
                por_modelo: IndexMap::new(),
                por_usd: Some(2.0),
                intervalos: 9,
            },
            seven: Taxas {
                por_modelo: IndexMap::new(),
                por_usd: None,
                intervalos: 0,
            },
        }
    }

    fn faixa(p25: f64, median: f64, p75: f64) -> Faixa {
        Faixa { p25, median, p75 }
    }

    #[test]
    fn dado_usd_do_plano_e_taxa_por_usd_entao_a_faixa_em_pct_sai_da_taxa_sem_taxa_sem_faixa() {
        let c = cal();
        assert_eq!(
            pct_de_usd(&c.five, Some(faixa(1.0, 2.0, 3.0))),
            Some(faixa(2.0, 4.0, 6.0))
        );
        assert_eq!(pct_de_usd(&c.seven, Some(faixa(1.0, 2.0, 3.0))), None);
        assert_eq!(pct_de_usd(&c.five, None), None);
    }

    #[test]
    fn dado_unidade_sem_custo_estimado_entao_a_soma_e_piso_pode_dizer_que_nao_cabe_nunca_que_cabe()
    {
        let unidades = [
            UnidadeCusto {
                unit: "a".into(),
                cost_usd: Some(faixa(10.0, 20.0, 30.0)),
            },
            UnidadeCusto {
                unit: "b".into(),
                cost_usd: None,
            },
        ];
        let justo = limites_do_plano(
            &unidades,
            &cal(),
            &Atuais {
                five: Some(atual(50.0)),
                seven: None,
            },
        );
        assert_eq!(justo.five.plano.unwrap().p75, 60.0);
        assert!(!justo.five.completo);
        assert_eq!(justo.five.cabe, Some(false));
        let folga = limites_do_plano(
            &unidades,
            &cal(),
            &Atuais {
                five: Some(atual(0.0)),
                seven: None,
            },
        );
        assert_eq!(folga.five.cabe, None);
        assert_eq!(folga.seven.plano, None);
    }

    #[test]
    fn dado_plano_completo_e_folga_entao_cabe_e_o_json_segue_a_ordem_do_ts() {
        let unidades = [UnidadeCusto {
            unit: "a".into(),
            cost_usd: Some(faixa(1.0, 2.0, 3.0)),
        }];
        let r = limites_do_plano(
            &unidades,
            &cal(),
            &Atuais {
                five: Some(atual(10.0)),
                seven: None,
            },
        );
        assert_eq!(r.five.cabe, Some(true));
        let v = r.five.to_value();
        let chaves: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            chaves,
            ["porUnidade", "plano", "completo", "atual", "resta", "cabe"]
        );
        let a = r.five.atual.unwrap().to_value();
        let chaves: Vec<&str> = a.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(chaves, ["pct", "resetsAt", "at"]);
    }

    #[test]
    fn dado_soma_de_faixas_vazia_entao_none() {
        assert_eq!(soma_faixas(&[]), None);
    }
}

mod borda {
    use super::*;

    fn home() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::empty()
            .with("HOME", dir.path().to_str().unwrap())
            .with("HADES_NOW", "2026-11-01T10:00:00Z");
        (dir, env)
    }

    fn escreve_amostras(dir: &tempfile::TempDir, linhas: &[String]) {
        let file = dir.path().join(".claude/hades/limits/samples.jsonl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("{}\n", linhas.join("\n"))).unwrap();
    }

    fn linha(s: i64, five: f64, seven: f64, fecha: i64) -> String {
        json!({"at": at(s), "five": {"pct": five, "resetsAt": fecha},
               "seven": {"pct": seven, "resetsAt": fecha + 4 * 86_400}})
        .to_string()
    }

    #[test]
    fn dado_nenhuma_variavel_entao_valem_os_padroes() {
        let (dir, env) = home();
        let k = chaves(&env, dir.path());
        assert_eq!(k.gap_s, 60.0);
        assert_eq!(k.retention_days, 30.0);
        assert_eq!(k.opcoes, OPCOES);
    }

    #[test]
    fn dado_valor_fora_da_faixa_entao_so_avisa_e_usa_o_padrao() {
        let (dir, env) = home();
        let env = env
            .with("HADES_LIMITS_MIN_INTERVALS", "1")
            .with("HADES_LIMITS_RETENTION_DAYS", "abc")
            .with("HADES_LIMITS_CACHE_READ_WEIGHT_PCT", "25")
            .with("HADES_LIMITS_SAMPLE_GAP_S", "0");
        let k = chaves(&env, dir.path());
        assert_eq!(k.opcoes.min_intervals, 5.0);
        assert_eq!(k.retention_days, 30.0);
        assert_eq!(k.opcoes.cache_read_weight, 0.25);
        assert_eq!(k.gap_s, 0.0);
    }

    #[test]
    fn dado_sem_amostras_entao_nao_le_transcript_nem_cria_pasta() {
        let (dir, env) = home();
        let agora = limits_now(&env, dir.path()).unwrap();
        assert_eq!(agora.amostras, 0);
        assert_eq!(agora.atual, Atuais::default());
        assert_eq!(agora.calibracao.five.intervalos, 0);
        assert!(!dir.path().join(".claude").exists());
    }

    #[test]
    fn dado_amostras_que_bastam_entao_calibra_pelos_transcripts_sem_gravar_o_cache() {
        let (dir, env) = home();
        let base = T0 - 3 * 3600;
        let fecha = T0 + 3 * 3600;
        let mut linhas = vec![linha(base, 0.0, 0.0, fecha)];
        let mut turnos = Vec::new();
        for i in 1..=8 {
            turnos.push(
                json!({"type": "assistant", "requestId": format!("q{i}"), "timestamp": at(base + i * 600 - 300),
                       "sessionId": "s", "cwd": "/x",
                       "message": {"model": "claude-opus-5-5",
                                   "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
                .to_string(),
            );
            linhas.push(linha(base + i * 600, (i * 2) as f64, i as f64, fecha));
        }
        escreve_amostras(&dir, &linhas);
        let projeto = dir.path().join(".claude/projects/p");
        fs::create_dir_all(&projeto).unwrap();
        fs::write(projeto.join("s.jsonl"), format!("{}\n", turnos.join("\n"))).unwrap();
        let agora = limits_now(&env, dir.path()).unwrap();
        assert_eq!(agora.amostras, 9);
        assert_eq!(agora.calibracao.five.intervalos, 8);
        assert_eq!(agora.calibracao.seven.intervalos, 8);
        close(agora.calibracao.five.por_modelo["opus-5-5"], 2.0, 1e-6);
        close(agora.calibracao.seven.por_modelo["opus-5-5"], 1.0, 1e-6);
        assert_eq!(agora.atual.five.as_ref().unwrap().pct, 16.0);
        assert_eq!(agora.atual.seven.as_ref().unwrap().pct, 8.0);
        assert!(!dir.path().join(".claude/hades/usage").exists());
    }

    #[test]
    fn dado_transcript_que_cresce_entao_o_cache_reparseia_so_o_que_mudou() {
        let (dir, env) = home();
        let projeto = dir.path().join(".claude/projects/p");
        fs::create_dir_all(&projeto).unwrap();
        let arquivo = projeto.join("s.jsonl");
        let turno = |id: &str, s: i64| {
            json!({"type": "assistant", "requestId": id, "timestamp": at(s), "sessionId": "s", "cwd": "/x",
                   "message": {"model": "claude-opus-5-5",
                               "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
            .to_string()
        };
        fs::write(&arquivo, format!("{}\n", turno("a", T0 - 300))).unwrap();
        let rules = GuardRules::load(&env).unwrap();
        let antes = requisicoes_do_arquivo(&arquivo, &rules, 0.0);
        assert_eq!(antes.len(), 1);
        assert_eq!(requisicoes_do_arquivo(&arquivo, &rules, 0.0), antes);
        fs::write(
            &arquivo,
            format!("{}\n{}\n", turno("a", T0 - 300), turno("b", T0 - 200)),
        )
        .unwrap();
        assert_eq!(requisicoes_do_arquivo(&arquivo, &rules, 0.0).len(), 2);
    }

    #[test]
    fn dado_poucas_amostras_entao_mostra_o_medido_e_a_calibracao_fica_sem_taxa() {
        let (dir, env) = home();
        let fecha = T0 + 3 * 3600;
        escreve_amostras(
            &dir,
            &[
                linha(T0 - 600, 20.0, 5.0, fecha),
                linha(T0, 23.0, 6.0, fecha),
            ],
        );
        let projeto = dir.path().join(".claude/projects/p");
        fs::create_dir_all(&projeto).unwrap();
        fs::write(
            projeto.join("s.jsonl"),
            json!({"type": "assistant", "requestId": "a", "timestamp": at(T0 - 300), "sessionId": "s",
                   "cwd": "/x", "message": {"model": "claude-opus-5-5",
                   "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
            .to_string(),
        )
        .unwrap();
        let agora = limits_now(&env, dir.path()).unwrap();
        assert!(agora.calibracao.five.por_modelo.is_empty());
        assert_eq!(agora.calibracao.five.intervalos, 1);
        assert_eq!(linha_atual(&agora.atual).split(" · ").count(), 2);
    }

    #[test]
    fn dado_amostra_fora_da_retencao_entao_nao_conta() {
        let (dir, env) = home();
        let fecha = T0 + 3 * 3600;
        escreve_amostras(&dir, &[linha(T0 - 40 * 86_400, 20.0, 5.0, fecha)]);
        let m = medidas(&env, (T0 * 1000) as f64, &chaves(&env, dir.path()));
        assert!(m.amostras.is_empty());
    }

    #[test]
    fn dado_renovacao_entao_a_hora_segue_o_formato_da_janela() {
        let epoch = 1_793_527_200.0 + 12.0 * 3600.0 + 600.0; // 2026-11-01T22:10:00Z
        assert_eq!(renova(Janela::Five, epoch), "22:10Z");
        assert_eq!(
            renova(Janela::Seven, epoch + 6.0 * 86_400.0 + 3000.0),
            "11-07 23:00Z"
        );
    }

    #[test]
    fn dado_pct_com_meio_entao_a_linha_arredonda_para_cima_como_o_math_round() {
        let a = Atuais {
            five: Some(Atual {
                pct: 23.5,
                resets_at: 1_793_527_200.0 + 3600.0,
                at: at(T0),
            }),
            seven: Some(Atual {
                pct: 41.4,
                resets_at: 1_793_527_200.0 + 86_400.0,
                at: at(T0),
            }),
        };
        assert_eq!(
            linha_atual(&a),
            "5 h 24% (renova 11:00Z) · semana 41% (renova 11-02 10:00Z)"
        );
        assert_eq!(linha_atual(&Atuais::default()), "");
    }
}
