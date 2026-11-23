//! `MoiraiView`: casca (topo, rail, status) e as telas Estimar plano e Histórico.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_component::input::{InputEvent, InputState};
use hades_core::Env;
use hades_ui::components::{
    Align, Button, Card, Confirm, DataTable, EmptyState, FilterBar, Kpi, KpiStrip, MetaList, Meter,
    Page, PageHeader, PathField, Rail, RailBrand, RailGroup, RailItem, RailStat, Segmented,
    SettingOrigin, SettingRow, SettingsBar, SettingsGroup, Skeleton, SortDirection, Spinner,
    StatusBadge, StatusBar, TermBlock,
};
use hades_ui::tokens::space;
use hades_ui::{IconName, palette};
use moirai_core::budget::{BudgetLevel, MoiraiRoutes};
use moirai_core::limits::{JANELAS, LimitesDoPlano};
use moirai_core::model::{HistoryUnit, Mode, Params};
use notify::{RecursiveMode, Watcher};

use crate::config::{self, ConfigLocal, FonteDeConfig, Linha, Origem};
use crate::service::{self, Estimativa};
use crate::vm::{
    self, Assinatura, COLUNAS_ESTIMAR, COLUNAS_HISTORICO, COLUNAS_TETO, Chip, Direcao, Filtros,
    Ordem, Tela,
};

/// Linhas da página Configurações e um campo de texto por linha.
struct CfgEstado {
    linhas: Vec<Linha>,
    entradas: Vec<Entity<InputState>>,
    erro: Option<String>,
}

pub struct MoiraiView {
    env: Env,
    fonte: Rc<dyn FonteDeConfig>,
    cfg: Option<CfgEstado>,
    cfg_salvo: bool,
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
    recolhida: bool,
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
            fonte: Rc::new(ConfigLocal::new(&env, &cwd)),
            cfg: None,
            cfg_salvo: false,
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
            recolhida: false,
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
        view.atualizar_janelas(cx);
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

    /// Esquerda: aviso ou estado; meio: fonte do histórico; ponta: versão (`StatusBar::version`).
    fn rodape(&self) -> (String, String, String) {
        let ocupado = self.t_historico.carregando()
            || self.t_estimar.carregando()
            || self.t_teto.carregando()
            || self.t_janelas.carregando()
            || self.profunda.estimando();
        let esquerda = self
            .aviso
            .clone()
            .unwrap_or_else(|| if ocupado { "calculando…" } else { "pronto" }.to_owned());
        (
            esquerda,
            format!("fonte: {}", vm::unidades_encerradas(self.historico.len())),
            format!("moirai {}", env!("CARGO_PKG_VERSION")),
        )
    }

    fn rail(&self, cx: &mut Context<Self>) -> Rail {
        let entidade = cx.entity();
        let item = |t: Tela, sub: &'static str| {
            let item = RailItem::new(t.rotulo()).icon(t.icone()).subtitle(sub);
            match t {
                Tela::Historico => item.badge(self.historico.len().to_string()),
                Tela::Teto if !self.teto.is_empty() => {
                    let dentro = self
                        .teto
                        .iter()
                        .filter(|l| matches!(l.nivel, Some(BudgetLevel::Ok)))
                        .count();
                    item.count(dentro, self.teto.len())
                }
                _ => item,
            }
        };
        let plano = RailGroup::new(
            "Plano",
            [
                item(Tela::Estimar, "custo e duração"),
                item(Tela::Historico, "unidades encerradas"),
            ],
        );
        let medicao = RailGroup::new(
            "Medição",
            [
                item(Tela::Teto, "custo real contra o p75"),
                item(Tela::Janelas, "limites do plano"),
            ],
        );
        let ajustes = RailGroup::new("Ajustes", [item(Tela::Config, "chaves HADES_MOIRAI_*")]);
        Rail::new([plano, medicao, ajustes])
            .brand(RailBrand::new("Moirai").subtitle("planos e estimativas"))
            .stat(self.rodape_do_rail())
            .theme_toggle()
            .collapsed(self.recolhida)
            .on_collapse({
                let entidade = entidade.clone();
                move |recolhida, _, cx| {
                    entidade.update(cx, |v, cx| {
                        v.recolhida = recolhida;
                        cx.notify();
                    });
                }
            })
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

    /// Janela mais curta medida: é o teto que o dono mais olha ao aprovar um plano.
    fn rodape_do_rail(&self) -> RailStat {
        let limites = self
            .janelas
            .as_ref()
            .or_else(|| self.estimativa.as_ref().map(|e| &e.limites));
        let Some(limites) = limites else {
            return RailStat::new("Janela de 5 h", "sem medida");
        };
        let j = JANELAS[0];
        let m = vm::medidor(j, limites.get(j));
        RailStat::new(format!("Janela de {}", j.rotulo()), m.texto)
            .fraction(m.fracao)
            .tone(m.tom)
    }

    fn tela_estimar(&self, cx: &mut Context<Self>) -> Page {
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
                .value(caminho)
                .placeholder("Escolha um plan.md")
                .prompt("Abrir plano")
                .on_pick(move |p, cx| {
                    e.update(cx, |v, cx| {
                        v.abrir_plano(p, cx);
                        cx.notify();
                    });
                })
        };
        let cabecalho = PageHeader::new("Estimar plano")
            .subtitle("Custo e duração por unidade do plano")
            .description(
                "A estimativa vem do histórico de unidades encerradas; gravar no plano troca a seção \"## Estimativa\".",
            )
            .action(seletor)
            .action(deep)
            .action(gravar);

        let pagina = Page::new("pagina-estimar").header(cabecalho).child(campo);
        let Some(est) = &self.estimativa else {
            return pagina.child(EmptyState::new("Nenhum plano aberto").fill().hint(
                "Escolha um plan.md com a tabela \"## Unidades\" para ver custo e duração por unidade.",
            ));
        };
        let k = vm::kpis(&est.estimativas, est.base);
        let pagina = pagina.child(KpiStrip::new([
            Kpi::new("Custo p50", k.custo_p50.map_or("—".into(), vm::usd)),
            Kpi::new("Custo p75", k.custo_p75.map_or("—".into(), vm::usd)),
            Kpi::new("Duração p50", vm::duracao_celula(k.duracao_p50_min)),
            Kpi::new("Base", format!("{} un.", k.base)),
        ]));

        let mut linhas = vm::linhas_estimar(&est.estimativas);
        vm::ordenar_estimar(&mut linhas, self.ordem_estimar);
        let mut tabela = DataTable::new(colunas(&COLUNAS_ESTIMAR)).fill("tabela-estimar");
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

        let mut janelas = Card::new()
            .title("Janelas do plano")
            .meta("uso atual e quanto o plano acrescenta");
        for j in JANELAS {
            let m = vm::medidor(j, est.limites.get(j));
            janelas = janelas.child(
                Meter::new(m.fracao)
                    .tone(m.tom)
                    .label(format!("Janela de {}", j.rotulo()))
                    .value(m.texto),
            );
        }
        pagina
            .child(
                Card::new()
                    .title("Unidades")
                    .meta(format!("{} no plano", linhas.len()))
                    .fill()
                    .child(tabela),
            )
            .child(janelas)
    }

    fn pedir_confirmacao(&mut self, cx: &mut Context<Self>) {
        self.confirmando = true;
        cx.notify();
    }

    fn tela_historico(&self, cx: &mut Context<Self>) -> Page {
        let entidade = cx.entity();
        let cabecalho = PageHeader::new("Histórico")
            .subtitle("Unidades encerradas com custo medido")
            .description("Os chips filtram por papel, modo e faixa; a estimativa usa só o que passa nos filtros.");
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
        let pagina = Page::new("pagina-historico").header(cabecalho).child(barra);
        let visiveis = vm::visiveis(&self.historico, &self.filtros, self.ordem_historico);
        if visiveis.is_empty() {
            let dica = if self.historico.is_empty() {
                "Nenhuma unidade encerrada com custo medido."
            } else {
                "Nenhuma unidade passa nos filtros; desligue um chip."
            };
            return pagina.child(EmptyState::new("Sem unidades").hint(dica).fill());
        }
        let mut tabela = DataTable::new(colunas(&COLUNAS_HISTORICO)).fill("tabela-historico");
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
        pagina.child(
            Card::new()
                .title("Unidades")
                .meta(format!("{} de {}", visiveis.len(), self.historico.len()))
                .fill()
                .child(tabela),
        )
    }

    fn tela_teto(&self, cx: &mut Context<Self>) -> Page {
        let entidade = cx.entity();
        let mut cabecalho = PageHeader::new("Teto ao vivo")
            .subtitle("Custo real das unidades em execução contra o p75 estimado")
            .description(
                "Unidades de um plano aprovado aparecem aqui assim que o transcript ganha custo.",
            );
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
        let pagina = Page::new("pagina-teto").header(cabecalho);
        if self.teto.is_empty() && self.t_teto.carregando() {
            return pagina.child(estado_carregando(
                "teto",
                "Lendo os transcripts das unidades em execução.",
            ));
        }
        if self.teto.is_empty() {
            return pagina.child(EmptyState::new("Nenhuma unidade em execução").fill().hint(
                "Unidades de um plano aprovado aparecem aqui com o custo real contra o p75 estimado.",
            ));
        }
        let mut tabela = DataTable::new(colunas(&COLUNAS_TETO)).fill("tabela-teto");
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
        let mut medidores = Card::new()
            .title("Uso do p75")
            .meta(vm::mais_recentes(self.teto.len().min(8)));
        for l in self.teto.iter().take(8) {
            let fracao = l.pct.map_or(0.0, |p| (p / 100.0).clamp(0.0, 1.0) as f32);
            medidores = medidores.child(
                Meter::new(fracao)
                    .tone(vm::tom_do_nivel(l.nivel))
                    .label(l.unidade.clone())
                    .value(vm::rotulo_do_nivel(l.nivel)),
            );
        }
        pagina.child(medidores).child(
            Card::new()
                .title("Unidades em execução")
                .meta(format!("{} no total", self.teto.len()))
                .fill()
                .child(tabela),
        )
    }

    /// Cria os campos da página na primeira vez que ela aparece (e depois de salvar ou descartar).
    fn garantir_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cfg.is_some() {
            return;
        }
        let (linhas, erro) = match self.fonte.carregar() {
            Ok(l) => (l, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let mut entradas = Vec::new();
        for l in &linhas {
            let valor = l.valor.clone();
            let entrada = cx.new(|cx| InputState::new(window, cx).default_value(valor));
            cx.subscribe(&entrada, |v: &mut Self, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    v.cfg_salvo = false;
                    cx.notify();
                }
            })
            .detach();
            entradas.push(entrada);
        }
        self.cfg = Some(CfgEstado {
            linhas,
            entradas,
            erro,
        });
    }

    fn textos_config(&self, cx: &App) -> Vec<String> {
        self.cfg.as_ref().map_or_else(Vec::new, |c| {
            c.entradas
                .iter()
                .map(|e| e.read(cx).value().to_string())
                .collect()
        })
    }

    fn salvar_config(&mut self, cx: &mut Context<Self>) {
        let textos = self.textos_config(cx);
        let Some(c) = &self.cfg else { return };
        match config::salvar(self.fonte.as_ref(), &c.linhas, &textos) {
            Ok(n) => {
                self.aviso = Some(format!("Configurações gravadas ({n})"));
                self.cfg = None;
                self.cfg_salvo = true;
                self.recarregar_historico(cx);
            }
            Err(e) => self.aviso = Some(e),
        }
        cx.notify();
    }

    fn descartar_config(&mut self, cx: &mut Context<Self>) {
        self.cfg = None;
        self.cfg_salvo = false;
        cx.notify();
    }

    fn tela_config(&self, cx: &mut Context<Self>) -> Page {
        let cabecalho = PageHeader::new("Configurações")
            .subtitle("Chaves HADES_MOIRAI_* do catálogo do Hades")
            .description("Mesmos nomes, padrões, faixas e precedência do CLI: ambiente, depois o config.env, depois o padrão.");
        let pagina = Page::new("pagina-config").header(cabecalho);
        let Some(c) = &self.cfg else {
            return pagina;
        };
        if let Some(erro) = &c.erro {
            return pagina.child(
                EmptyState::new("Catálogo indisponível")
                    .hint(erro.clone())
                    .fill(),
            );
        }
        let textos = self.textos_config(cx);
        let erros = config::erros(&c.linhas, &textos);
        let pendentes = config::pendentes(&c.linhas, &textos);
        let entidade = cx.entity();
        let mut grupos: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, l) in c.linhas.iter().enumerate() {
            match grupos.iter_mut().find(|(g, _)| *g == l.grupo) {
                Some((_, idx)) => idx.push(i),
                None => grupos.push((l.grupo.clone(), vec![i])),
            }
        }
        let mut pagina = pagina;
        if !self.fonte.gravavel() {
            pagina = pagina.child(Card::new().note(
                "Sem instalação do Hades: a página mostra os padrões do kit e não grava nada.",
            ));
        }
        for (nome, indices) in grupos {
            let fora = indices
                .iter()
                .filter(|&&i| textos[i].trim() != c.linhas[i].padrao)
                .count();
            let mut grupo = SettingsGroup::new(nome).meta(format!(
                "{} · {fora} fora do padrão",
                vm::configuracoes(indices.len())
            ));
            for i in indices {
                let l = &c.linhas[i];
                let id = SharedString::from(format!("cfg-{}", l.chave));
                let entrada = &c.entradas[i];
                let mut linha = match l.faixa() {
                    Some((min, max)) => SettingRow::number(id, l.rotulo.clone(), entrada, min, max),
                    None => SettingRow::text(id, l.rotulo.clone(), entrada),
                }
                .key(l.chave.clone())
                .description(l.descricao.clone())
                .default_value(l.padrao.clone())
                .origin(match l.origem {
                    Origem::Padrao => SettingOrigin::Default,
                    Origem::Arquivo => SettingOrigin::File,
                    Origem::Ambiente => SettingOrigin::Env,
                })
                .modified(textos[i].trim() != l.padrao)
                .error_opt(erros[i].clone());
                let (entrada, padrao) = (entrada.clone(), l.padrao.clone());
                linha = linha.on_reset(move |_, window, cx| {
                    entrada.update(cx, |s, cx| s.set_value(padrao.clone(), window, cx));
                });
                grupo = grupo.row(linha);
            }
            pagina = pagina.child(grupo);
        }
        let (e_salvar, e_descartar) = (entidade.clone(), entidade);
        pagina.child(
            SettingsBar::new(pendentes)
                .saved(self.cfg_salvo)
                .on_save(move |_, _, cx| e_salvar.update(cx, |v, cx| v.salvar_config(cx)))
                .on_discard(move |_, _, cx| e_descartar.update(cx, |v, cx| v.descartar_config(cx))),
        )
    }

    fn tela_janelas(&self) -> Page {
        let cabecalho = PageHeader::new("Janelas")
            .subtitle("Limites do plano de assinatura medidos pelo status line")
            .description("Cada janela mostra o uso atual, quando renova e quanto o plano aberto deve acrescentar.");
        let pagina = Page::new("pagina-janelas").header(cabecalho);
        let Some(limites) = &self.janelas else {
            if self.t_janelas.carregando() {
                return pagina.child(estado_carregando(
                    "janelas",
                    "Relendo os transcripts para calibrar as janelas; a primeira leitura pode levar um minuto.",
                ));
            }
            return pagina.child(EmptyState::new("Sem medida").fill().hint(
                "Nenhuma leitura de janela do status line ainda; abra uma sessão do Claude Code.",
            ));
        };
        let mut cartoes = div().flex().gap(px(space::LG));
        let mut atuais = Vec::new();
        for j in JANELAS {
            let limite = limites.get(j);
            let c = vm::cartao_da_janela(j, limite, self.agora_ms);
            atuais.push((j, limite.atual.as_ref()));
            cartoes = cartoes.child(
                div().flex_1().min_w_0().child(
                    Card::new()
                        .title(format!("Janela de {}", j.rotulo()))
                        .child(
                            Meter::new(c.medidor.fracao)
                                .tone(c.medidor.tom)
                                .label("Uso")
                                .value(c.medidor.texto),
                        )
                        .child(
                            MetaList::new()
                                .row("Renovação", c.renovacao)
                                .row("Plano aberto", c.plano),
                        ),
                ),
            );
        }
        let linha = vm::linha_das_janelas(&atuais);
        pagina.child(cartoes).child(
            Card::new()
                .title("Linha de status")
                .meta("a mesma que o terminal mostra")
                .child(
                    TermBlock::new()
                        .line(linha.clone())
                        .copy("copiar-janelas", linha),
                ),
        )
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        window.set_window_title(&self.titulo());
        if self.tela == Tela::Config {
            self.garantir_config(window, cx);
        }
        let (esquerda, fonte, versao) = self.rodape();
        let rail = self.rail(cx);
        let corpo = match self.tela {
            Tela::Estimar => self.tela_estimar(cx),
            Tela::Historico => self.tela_historico(cx),
            Tela::Teto => self.tela_teto(cx),
            Tela::Janelas => self.tela_janelas(),
            Tela::Config => self.tela_config(cx),
        };
        let mut raiz = div()
            .id("moirai")
            .track_focus(&self.foco)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if v.tela == Tela::Config {
                    return;
                }
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
            .child(div().flex_1().flex().min_h_0().child(rail).child(corpo))
            .child(StatusBar::new(esquerda, fonte).version(versao));
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
