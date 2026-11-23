//! `MoiraiView`: casca (topo, rail, status) e as telas Estimar plano e Histórico.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};
use hades_core::Env;
use hades_ui::components::{
    Align, Button, Card, Confirm, DataTable, EmptyState, FilterBar, Kpi, KpiStrip, Meter,
    PageHeader, PathField, Rail, RailBrand, RailGroup, RailItem, Segmented, Skeleton,
    SortDirection, Spinner, StatusBadge, StatusBar, TermBlock, TopBar,
};
use hades_ui::tokens::space;
use hades_ui::{IconName, palette};
use moirai_core::budget::MoiraiRoutes;
use moirai_core::limits::{JANELAS, LimitesDoPlano};
use moirai_core::model::{HistoryUnit, Mode, Params};
use notify::{RecursiveMode, Watcher};

use crate::service::{self, Estimativa};
use crate::vm::{
    self, Assinatura, COLUNAS_ESTIMAR, COLUNAS_HISTORICO, COLUNAS_TETO, Chip, Direcao, Filtros,
    Ordem, Tela,
};

pub struct MoiraiView {
    env: Env,
    cwd: PathBuf,
    tela: Tela,
    params: Option<Params>,
    historico: Vec<HistoryUnit>,
    plano: Option<PathBuf>,
    modo: Mode,
    estimativa: Option<Estimativa>,
    selecao_estimar: Option<usize>,
    ordem_estimar: Option<Ordem>,
    filtros: Filtros,
    ordem_historico: Option<Ordem>,
    selecao_historico: Option<usize>,
    confirmando: bool,
    aviso: Option<String>,
    foco: FocusHandle,
    painel: Arc<Mutex<MoiraiRoutes>>,
    teto: Vec<vm::LinhaTeto>,
    limiares: Option<(f64, f64)>,
    selecao_teto: Option<usize>,
    janelas: Option<LimitesDoPlano>,
    agora_ms: f64,
    sujo: Arc<AtomicBool>,
    observador: Option<notify::RecommendedWatcher>,
    assinatura: Option<Assinatura>,
    tique: u32,
    t_historico: vm::Tarefa,
    t_estimar: vm::Tarefa,
    t_teto: vm::Tarefa,
    t_janelas: vm::Tarefa,
    profunda: vm::Profunda,
}

const PERIODO_MS: u64 = 500;
const TIQUES_TETO: u32 = 6;
const TIQUES_JANELAS: u32 = 60;

impl MoiraiView {
    pub fn new(env: Env, cwd: PathBuf, foco: FocusHandle, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            tela: Tela::Estimar,
            params: None,
            historico: Vec::new(),
            plano: None,
            modo: Mode::Projeto,
            estimativa: None,
            selecao_estimar: None,
            ordem_estimar: None,
            filtros: Filtros::default(),
            ordem_historico: None,
            selecao_historico: None,
            confirmando: false,
            aviso: None,
            foco,
            painel: Arc::new(Mutex::new(service::novo_painel(&env, &cwd))),
            teto: Vec::new(),
            limiares: None,
            selecao_teto: None,
            janelas: None,
            agora_ms: 0.0,
            sujo: Arc::new(AtomicBool::new(false)),
            observador: None,
            assinatura: None,
            tique: 0,
            t_historico: vm::Tarefa::default(),
            t_estimar: vm::Tarefa::default(),
            t_teto: vm::Tarefa::default(),
            t_janelas: vm::Tarefa::default(),
            profunda: vm::Profunda::default(),
            env,
            cwd,
        };
        view.recarregar_historico(cx);
        view
    }

    /// Lê parâmetros e histórico no executor de fundo; ao terminar, reestima o plano aberto.
    fn recarregar_historico(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.t_historico.iniciar() else {
            return;
        };
        let (env, cwd) = (self.env.clone(), self.cwd.clone());
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let params = service::carregar_params(&env, &cwd)?;
                    let historico = service::carregar_historico(&env, &cwd, &params)?;
                    Ok::<_, hades_core::output::HadesError>((params, historico))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_historico.concluir(g) {
                    return;
                }
                match r {
                    Ok((params, historico)) => {
                        v.params = Some(params);
                        v.historico = historico;
                        v.reestimar(cx);
                    }
                    Err(e) => v.aviso = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Estima no executor de fundo; mudar de plano ou de modo invalida a estimativa em voo.
    fn reestimar(&mut self, cx: &mut Context<Self>) {
        self.t_estimar.invalidar();
        let (Some(plano), Some(params)) = (self.plano.clone(), self.params) else {
            return;
        };
        let Some(g) = self.t_estimar.iniciar() else {
            return;
        };
        let (env, cwd, modo, historico, dicas) = (
            self.env.clone(),
            self.cwd.clone(),
            self.modo,
            self.historico.clone(),
            self.profunda.dicas().clone(),
        );
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    service::estimar(&env, &cwd, &plano, modo, &params, &historico, &dicas)
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_estimar.concluir(g) {
                    return;
                }
                match r {
                    Ok(e) => {
                        v.estimativa = Some(e);
                        v.selecao_estimar = None;
                        v.aviso = None;
                        v.atualizar_janelas(cx);
                    }
                    Err(e) => {
                        v.estimativa = None;
                        v.aviso = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Estimativa profunda no executor de fundo; ao voltar, reestima o plano com as dicas.
    fn rodar_profunda(&mut self, cx: &mut Context<Self>) {
        let Some(plano) = self.plano.clone() else {
            return;
        };
        let Some(g) = self.profunda.iniciar() else {
            return;
        };
        self.aviso = None;
        let env = self.env.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { service::profunda(&env, &plano).map_err(|e| e.to_string()) })
                .await;
            let _ = this.update(cx, |v, cx| {
                match v.profunda.concluir(g, r) {
                    vm::Profundo::Descartado => return,
                    vm::Profundo::Aplicado => v.reestimar(cx),
                    vm::Profundo::Falhou(msg) => v.aviso = Some(msg),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn abrir_plano(&mut self, caminho: PathBuf, cx: &mut Context<Self>) {
        self.profunda.esquecer();
        self.modo = service::modo_padrao(&self.env, &caminho);
        self.observar(&caminho);
        self.plano = Some(caminho);
        self.reestimar(cx);
    }

    /// Vigia a pasta do plano: toda gravação (do editor ou do próprio app) marca `sujo`.
    fn observar(&mut self, caminho: &std::path::Path) {
        self.observador = None;
        let Some(pasta) = caminho.parent() else {
            return;
        };
        let sujo = Arc::clone(&self.sujo);
        let alvo = caminho.file_name().map(std::ffi::OsStr::to_owned);
        let observador = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            if let Ok(ev) = ev
                && ev.paths.iter().any(|p| p.file_name() == alvo.as_deref())
            {
                sujo.store(true, Ordering::Release);
            }
        });
        match observador {
            Ok(mut w) => match w.watch(pasta, RecursiveMode::NonRecursive) {
                Ok(()) => self.observador = Some(w),
                Err(e) => self.aviso = Some(format!("sem releitura automática: {e}")),
            },
            Err(e) => self.aviso = Some(format!("sem releitura automática: {e}")),
        }
        self.assinatura = service::assinatura(caminho);
    }

    /// Relê o plano se a gravação mudou o arquivo de fato (tamanho ou mtime).
    fn reler_se_mudou(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.sujo.swap(false, Ordering::AcqRel) {
            return false;
        }
        let Some(plano) = self.plano.clone() else {
            return false;
        };
        let agora = service::assinatura(&plano);
        if !vm::mudou(self.assinatura, agora) {
            return false;
        }
        self.assinatura = agora;
        self.profunda.esquecer();
        self.reestimar(cx);
        true
    }

    fn atualizar_teto(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.t_teto.iniciar() else {
            return;
        };
        let painel = Arc::clone(&self.painel);
        cx.spawn(async move |this, cx| {
            let (teto, limiares) = cx
                .background_executor()
                .spawn(async move {
                    let plano = painel.lock().unwrap_or_else(PoisonError::into_inner).plan();
                    (vm::linhas_teto(&plano), vm::limiares(&plano))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.t_teto.concluir(g) {
                    v.teto = teto;
                    v.limiares = limiares;
                    v.selecao_teto = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn atualizar_janelas(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.t_janelas.iniciar() else {
            return;
        };
        let (env, cwd) = (self.env.clone(), self.cwd.clone());
        let estimativas = self
            .estimativa
            .as_ref()
            .map_or_else(Vec::new, |e| e.estimativas.clone());
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move {
                    let agora = service::agora_ms(&env);
                    service::janelas(&env, &cwd, &estimativas).map(|j| (agora, j))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if !v.t_janelas.concluir(g) {
                    return;
                }
                match r {
                    Ok((agora, j)) => {
                        v.agora_ms = agora;
                        v.janelas = Some(j);
                    }
                    Err(e) => v.aviso = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Um tique do laço de fundo: relê o plano gravado e dispara a renovação do Teto ao vivo e das Janelas.
    fn tiquetaque(&mut self, cx: &mut Context<Self>) {
        self.tique = self.tique.wrapping_add(1);
        self.reler_se_mudou(cx);
        if self.tela == Tela::Teto && self.tique.is_multiple_of(TIQUES_TETO) {
            self.atualizar_teto(cx);
        }
        if self.tela == Tela::Janelas && self.tique.is_multiple_of(TIQUES_JANELAS) {
            self.atualizar_janelas(cx);
        }
    }

    fn mudar_tela(&mut self, tela: Tela, cx: &mut Context<Self>) {
        self.tela = tela;
        match tela {
            Tela::Teto => self.atualizar_teto(cx),
            Tela::Janelas => self.atualizar_janelas(cx),
            _ => {}
        }
    }

    pub fn iniciar(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(PERIODO_MS))
                    .await;
                let vivo = this.update(cx, |v, cx| v.tiquetaque(cx));
                if vivo.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn gravar(&mut self) {
        let (Some(e), Some(params)) = (self.estimativa.as_ref(), self.params) else {
            return;
        };
        self.aviso = Some(
            match service::gravar_no_plano(&self.env, &self.cwd, e, params) {
                Ok(true) => {
                    "Seção \"## Estimativa\" substituída; aprove o plano de novo.".to_owned()
                }
                Ok(false) => "Seção \"## Estimativa\" gravada; aprove o plano de novo.".to_owned(),
                Err(err) => err.to_string(),
            },
        );
        if let Some(p) = &self.plano {
            self.assinatura = service::assinatura(p);
        }
        self.sujo.store(false, Ordering::Release);
    }

    fn titulo(&self) -> String {
        match &self.plano {
            Some(p) if self.tela == Tela::Estimar => format!("Moirai · {}", p.display()),
            _ => "Moirai".to_owned(),
        }
    }

    fn rodape(&self) -> (String, String) {
        let ocupado = self.t_historico.carregando()
            || self.t_estimar.carregando()
            || self.t_teto.carregando()
            || self.t_janelas.carregando()
            || self.profunda.estimando();
        let esquerda = self.aviso.clone().unwrap_or_else(|| {
            if ocupado {
                "calculando…".to_owned()
            } else {
                format!("fonte: {} unidades encerradas", self.historico.len())
            }
        });
        (esquerda, format!("moirai {}", env!("CARGO_PKG_VERSION")))
    }

    fn rail(&self, cx: &mut Context<Self>) -> Rail {
        let entidade = cx.entity();
        let item = |t: Tela| {
            let item = RailItem::new(t.rotulo()).icon(t.icone());
            if t == Tela::Historico {
                item.badge(self.historico.len().to_string())
            } else {
                item
            }
        };
        let plano = RailGroup::new("Plano", [item(Tela::Estimar), item(Tela::Historico)]);
        let medicao = RailGroup::new("Medição", [item(Tela::Teto), item(Tela::Janelas)]);
        Rail::new([plano, medicao])
            .brand(RailBrand::hades().subtitle("Moirai"))
            .selected(self.tela.rotulo())
            .on_select(move |rotulo, _, cx| {
                if let Some(t) = Tela::de_rotulo(rotulo) {
                    entidade.update(cx, |v, cx| {
                        v.mudar_tela(t, cx);
                        cx.notify();
                    });
                }
            })
    }

    fn tela_estimar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entidade = cx.entity();
        let caminho = self
            .plano
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        let pode_gravar = self.estimativa.is_some();
        let deep = if self.plano.is_none() {
            Button::new("profunda", "Profunda")
                .icon(IconName::ScanSearch)
                .disabled("Abra um plano para estimar em profundidade")
        } else if self.profunda.estimando() {
            Button::new("profunda", "Estimando…")
                .icon(IconName::ScanSearch)
                .loading(true)
                .disabled("A estimativa profunda está rodando")
        } else {
            let e = entidade.clone();
            Button::new("profunda", "Profunda")
                .icon(IconName::ScanSearch)
                .on_click(move |_, _, cx| e.update(cx, |v, cx| v.rodar_profunda(cx)))
        };
        let gravar = {
            let b = Button::new("gravar", "Gravar no plano")
                .icon(IconName::Save)
                .primary();
            if pode_gravar {
                let e = entidade.clone();
                b.on_click(move |_, _, cx| e.update(cx, |v, cx| v.pedir_confirmacao(cx)))
            } else {
                b.disabled("Abra um plano com tabela em \"## Unidades\"")
            }
        };
        let seletor = {
            let e = entidade.clone();
            Segmented::new("modo", ["projeto", "skill"])
                .selected(vm::indice_do_modo(self.modo))
                .on_select(move |i, _, cx| {
                    e.update(cx, |v, cx| {
                        v.modo = vm::modo_de_indice(i);
                        v.reestimar(cx);
                        cx.notify();
                    });
                })
        };
        let campo = {
            let e = entidade.clone();
            PathField::new("plano")
                .value(caminho.clone())
                .placeholder("Escolha um plan.md")
                .prompt("Abrir plano")
                .on_pick(move |p, cx| {
                    e.update(cx, |v, cx| {
                        v.abrir_plano(p, cx);
                        cx.notify();
                    });
                })
        };
        let cabecalho = PageHeader::new(if caminho.is_empty() {
            "Estimar plano".to_owned()
        } else {
            caminho
        })
        .action(seletor)
        .action(deep)
        .action(gravar);

        let mut coluna = div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(cabecalho)
            .child(campo);
        let Some(est) = &self.estimativa else {
            return coluna
                .child(EmptyState::new("Nenhum plano aberto").hint(
                    "Escolha um plan.md com a tabela \"## Unidades\" para ver custo e duração por unidade.",
                ))
                .into_any_element();
        };
        let k = vm::kpis(&est.estimativas, est.base);
        coluna = coluna.child(KpiStrip::new([
            Kpi::new("Custo p50", k.custo_p50.map_or("—".into(), vm::usd)),
            Kpi::new("Custo p75", k.custo_p75.map_or("—".into(), vm::usd)),
            Kpi::new("Duração p50", vm::duracao_celula(k.duracao_p50_min)),
            Kpi::new("Base", format!("{} un.", k.base)),
        ]));

        let mut linhas = vm::linhas_estimar(&est.estimativas);
        vm::ordenar_estimar(&mut linhas, self.ordem_estimar);
        let mut tabela = DataTable::new(colunas(&COLUNAS_ESTIMAR));
        for l in &linhas {
            tabela = tabela.row(vm::celulas_estimar(l).into_iter().map(SharedString::from));
        }
        if let Some(i) = self.selecao_estimar {
            tabela = tabela.selected(i);
        }
        if let Some(o) = self.ordem_estimar {
            tabela = tabela.sorted(o.coluna, direcao_ui(o.direcao));
        }
        let (e1, e2) = (entidade.clone(), entidade);
        tabela = tabela
            .on_select(move |i, _, cx| {
                e1.update(cx, |v, cx| {
                    v.selecao_estimar = Some(i);
                    cx.notify();
                });
            })
            .on_sort(move |c, _, cx| {
                e2.update(cx, |v, cx| {
                    v.ordem_estimar = Some(Ordem::alternar(v.ordem_estimar, c));
                    v.selecao_estimar = None;
                    cx.notify();
                });
            });
        coluna = coluna.child(tabela);

        let mut janelas = div().flex().gap(px(space::MD));
        for j in JANELAS {
            let m = vm::medidor(j, est.limites.get(j));
            janelas = janelas.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .child(format!("Janela de {}", j.rotulo()))
                    .child(Meter::new(m.fracao).tone(m.tom))
                    .child(m.texto),
            );
        }
        coluna.child(janelas).into_any_element()
    }

    fn pedir_confirmacao(&mut self, cx: &mut Context<Self>) {
        self.confirmando = true;
        cx.notify();
    }

    fn tela_historico(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entidade = cx.entity();
        let cabecalho = PageHeader::new("Histórico");
        let chips = vm::chips_do_historico(&self.historico);
        let mut barra = FilterBar::new().label("Filtrar");
        for c in &chips {
            barra = barra.chip(vm::rotulo_do_chip(c), vm::chip_ativo(&self.filtros, c));
        }
        let e = entidade.clone();
        let barra = barra.on_toggle(move |i, _, cx| {
            let chip: Chip = chips[i].clone();
            e.update(cx, |v, cx| {
                vm::alternar_chip(&mut v.filtros, &chip);
                v.selecao_historico = None;
                cx.notify();
            });
        });
        let coluna = div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(cabecalho)
            .child(barra);
        let visiveis = vm::visiveis(&self.historico, &self.filtros, self.ordem_historico);
        if visiveis.is_empty() {
            let dica = if self.historico.is_empty() {
                "Nenhuma unidade encerrada com custo medido."
            } else {
                "Nenhuma unidade passa nos filtros; desligue um chip."
            };
            return coluna
                .child(EmptyState::new("Sem unidades").hint(dica))
                .into_any_element();
        }
        let mut tabela = DataTable::new(colunas(&COLUNAS_HISTORICO));
        for &i in &visiveis {
            tabela = tabela.row(
                vm::celulas_historico(&self.historico[i])
                    .into_iter()
                    .map(SharedString::from),
            );
        }
        if let Some(i) = self.selecao_historico {
            tabela = tabela.selected(i);
        }
        if let Some(o) = self.ordem_historico {
            tabela = tabela.sorted(o.coluna, direcao_ui(o.direcao));
        }
        let (e1, e2) = (entidade.clone(), entidade);
        let tabela = tabela
            .on_select(move |i, _, cx| {
                e1.update(cx, |v, cx| {
                    v.selecao_historico = Some(i);
                    cx.notify();
                });
            })
            .on_sort(move |c, _, cx| {
                e2.update(cx, |v, cx| {
                    v.ordem_historico = Some(Ordem::alternar(v.ordem_historico, c));
                    v.selecao_historico = None;
                    cx.notify();
                });
            });
        coluna.child(tabela).into_any_element()
    }

    fn tela_teto(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entidade = cx.entity();
        let mut cabecalho = PageHeader::new("Teto ao vivo");
        if let Some((aviso, pergunta)) = self.limiares {
            cabecalho = cabecalho.action(StatusBadge::new(
                format!(
                    "aviso {}% · pergunta {}%",
                    vm::numero(aviso, 0),
                    vm::numero(pergunta, 0)
                ),
                hades_ui::Tone::Neutral,
            ));
        }
        let coluna = div().flex().flex_col().gap(px(space::LG)).child(cabecalho);
        if self.teto.is_empty() && self.t_teto.carregando() {
            return coluna
                .child(estado_carregando(
                    "teto",
                    "Lendo os transcripts das unidades em execução.",
                ))
                .into_any_element();
        }
        if self.teto.is_empty() {
            return coluna
                .child(EmptyState::new("Nenhuma unidade em execução").hint(
                    "Unidades de um plano aprovado aparecem aqui com o custo real contra o p75 estimado.",
                ))
                .into_any_element();
        }
        let mut tabela = DataTable::new(colunas(&COLUNAS_TETO));
        for l in &self.teto {
            tabela = tabela.row(vm::celulas_teto(l).into_iter().map(SharedString::from));
        }
        if let Some(i) = self.selecao_teto {
            tabela = tabela.selected(i);
        }
        let tabela = tabela.on_select(move |i, _, cx| {
            entidade.update(cx, |v, cx| {
                v.selecao_teto = Some(i);
                cx.notify();
            });
        });
        let mut medidores = div().flex().flex_col().gap(px(space::SM));
        for l in self.teto.iter().take(8) {
            let fracao = l.pct.map_or(0.0, |p| (p / 100.0).clamp(0.0, 1.0) as f32);
            medidores = medidores.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(div().w_1_4().child(l.unidade.clone()))
                    .child(
                        div()
                            .flex_1()
                            .child(Meter::new(fracao).tone(vm::tom_do_nivel(l.nivel))),
                    )
                    .child(StatusBadge::new(
                        vm::rotulo_do_nivel(l.nivel),
                        vm::tom_do_nivel(l.nivel),
                    )),
            );
        }
        coluna.child(medidores).child(tabela).into_any_element()
    }

    fn tela_janelas(&self) -> gpui::AnyElement {
        let cabecalho = PageHeader::new("Janelas");
        let coluna = div().flex().flex_col().gap(px(space::LG)).child(cabecalho);
        let Some(limites) = &self.janelas else {
            if self.t_janelas.carregando() {
                return coluna
                    .child(estado_carregando(
                        "janelas",
                        "Relendo os transcripts para calibrar as janelas; a primeira leitura pode levar um minuto.",
                    ))
                    .into_any_element();
            }
            return coluna
                .child(EmptyState::new("Sem medida").hint(
                    "Nenhuma leitura de janela do status line ainda; abra uma sessão do Claude Code.",
                ))
                .into_any_element();
        };
        let mut cartoes = div().flex().gap(px(space::MD));
        let mut atuais = Vec::new();
        for j in JANELAS {
            let limite = limites.get(j);
            let c = vm::cartao_da_janela(j, limite, self.agora_ms);
            atuais.push((j, limite.atual.as_ref()));
            cartoes = cartoes.child(
                div().flex_1().child(
                    Card::new()
                        .title(format!("Janela de {}", j.rotulo()))
                        .child(Meter::new(c.medidor.fracao).tone(c.medidor.tom))
                        .child(c.medidor.texto)
                        .child(c.renovacao)
                        .child(c.plano),
                ),
            );
        }
        let linha = vm::linha_das_janelas(&atuais);
        coluna
            .child(cartoes)
            .child(TermBlock::new().line(linha))
            .into_any_element()
    }
}

fn estado_carregando(id: &'static str, dica: &'static str) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(space::MD))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::SM))
                .child(Spinner::new(SharedString::from(format!("{id}-giro"))))
                .child(StatusBadge::new("Calculando", hades_ui::Tone::Busy))
                .child(dica),
        )
        .child(Skeleton::new(SharedString::from(format!("{id}-sk1"))))
        .child(Skeleton::new(SharedString::from(format!("{id}-sk2"))))
        .into_any_element()
}

fn direcao_ui(d: Direcao) -> SortDirection {
    match d {
        Direcao::Asc => SortDirection::Ascending,
        Direcao::Desc => SortDirection::Descending,
    }
}

fn colunas(nomes: &[&str]) -> Vec<(SharedString, Align)> {
    nomes
        .iter()
        .map(|n| {
            let alinhamento = if vm::coluna_numerica(n) {
                Align::Numeric
            } else {
                Align::Start
            };
            (SharedString::from((*n).to_owned()), alinhamento)
        })
        .collect()
}

impl Render for MoiraiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (esquerda, direita) = self.rodape();
        let rail = self.rail(cx);
        let corpo = match self.tela {
            Tela::Estimar => self.tela_estimar(cx),
            Tela::Historico => self.tela_historico(cx),
            Tela::Teto => self.tela_teto(cx),
            Tela::Janelas => self.tela_janelas(),
        };
        let mut raiz = div()
            .id("moirai")
            .track_focus(&self.foco)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if let Some(t) = vm::tela_da_tecla(&ev.keystroke.key) {
                    v.mudar_tela(t, cx);
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .child(TopBar::new(self.titulo()).trailing("gpui"))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .min_h_0()
                    .child(rail)
                    .child(div().flex_1().p(px(space::LG)).child(corpo)),
            )
            .child(StatusBar::new(esquerda, direita));
        if self.confirmando {
            let alvo = self
                .plano
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string());
            let (sim, nao) = (cx.entity(), cx.entity());
            raiz = raiz.child(
                Confirm::new("Gravar a estimativa no plano?")
                    .detail("A seção \"## Estimativa\" entra no hash da aprovação: o plano precisa ser aprovado de novo.")
                    .target(alvo)
                    .confirm_label("Gravar")
                    .on_confirm(move |_, _, cx| {
                        sim.update(cx, |v, cx| {
                            v.confirmando = false;
                            v.gravar();
                            cx.notify();
                        });
                    })
                    .on_cancel(move |_, _, cx| {
                        nao.update(cx, |v, cx| {
                            v.confirmando = false;
                            cx.notify();
                        });
                    }),
            );
        }
        raiz
    }
}

pub fn abrir_janela(cx: &mut App) {
    abrir_janela_em(cx, None);
}

/// Abre a janela já na `tela` pedida, ou em Estimar plano.
pub fn abrir_janela_em(cx: &mut App, tela: Option<Tela>) {
    let aberta = cx.open_window(gpui::WindowOptions::default(), |window, cx| {
        window.set_window_title(crate::NOME);
        let cwd = std::env::current_dir().unwrap_or_default();
        let view: Entity<MoiraiView> = cx.new(|cx| {
            MoiraiView::iniciar(cx);
            let mut v = MoiraiView::new(Env::from_process(), cwd, cx.focus_handle(), cx);
            let mut args = std::env::args();
            if let Some(plano) = args
                .by_ref()
                .skip_while(|a| a != "--plano")
                .nth(1)
                .map(PathBuf::from)
            {
                v.abrir_plano(plano, cx);
            }
            if let Some(t) = tela {
                v.mudar_tela(t, cx);
            }
            v
        });
        view
    });
    if aberta.is_err() {
        cx.quit();
    }
}
