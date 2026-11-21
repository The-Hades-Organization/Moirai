//! `MoiraiView`: casca (topo, rail, status) e as telas Estimar plano e Histórico.

use std::path::PathBuf;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};
use hades_core::Env;
use hades_ui::components::{
    Align, Button, Confirm, DataTable, EmptyState, FilterBar, Kpi, KpiStrip, Meter, PageHeader,
    PathField, Rail, RailGroup, RailItem, Segmented, SortDirection, StatusBar, TopBar,
};
use hades_ui::palette;
use moirai_core::limits::JANELAS;
use moirai_core::model::{HistoryUnit, Mode, Params};

use crate::service::{self, Estimativa};
use crate::vm::{self, COLUNAS_ESTIMAR, COLUNAS_HISTORICO, Chip, Direcao, Filtros, Ordem, Tela};

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
}

impl MoiraiView {
    pub fn new(env: Env, cwd: PathBuf, foco: FocusHandle) -> Self {
        let mut view = Self {
            env,
            cwd,
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
        };
        view.recarregar_historico();
        view
    }

    fn recarregar_historico(&mut self) {
        let params = match service::carregar_params(&self.env, &self.cwd) {
            Ok(p) => p,
            Err(e) => {
                self.aviso = Some(e.to_string());
                return;
            }
        };
        match service::carregar_historico(&self.env, &self.cwd, &params) {
            Ok(h) => self.historico = h,
            Err(e) => self.aviso = Some(e.to_string()),
        }
        self.params = Some(params);
    }

    fn reestimar(&mut self) {
        let (Some(plano), Some(params)) = (self.plano.clone(), self.params) else {
            return;
        };
        match service::estimar(
            &self.env,
            &self.cwd,
            &plano,
            self.modo,
            &params,
            &self.historico,
        ) {
            Ok(e) => {
                self.estimativa = Some(e);
                self.selecao_estimar = None;
                self.aviso = None;
            }
            Err(e) => {
                self.estimativa = None;
                self.aviso = Some(e.to_string());
            }
        }
    }

    fn abrir_plano(&mut self, caminho: PathBuf) {
        self.modo = service::modo_padrao(&self.env, &caminho);
        self.plano = Some(caminho);
        self.reestimar();
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
    }

    fn titulo(&self) -> String {
        match &self.plano {
            Some(p) if self.tela == Tela::Estimar => format!("Moirai · {}", p.display()),
            _ => "Moirai".to_owned(),
        }
    }

    fn rodape(&self) -> (String, String) {
        let esquerda = self
            .aviso
            .clone()
            .unwrap_or_else(|| format!("fonte: {} unidades encerradas", self.historico.len()));
        (esquerda, format!("moirai {}", env!("CARGO_PKG_VERSION")))
    }

    fn rail(&self) -> Rail {
        let itens = vm::Tela::TODAS.map(|t| {
            let item = RailItem::new(t.rotulo());
            if t == Tela::Historico {
                item.badge(self.historico.len().to_string())
            } else {
                item
            }
        });
        Rail::new([RailGroup::new("Moirai", itens)]).selected(self.tela.rotulo())
    }

    fn tela_estimar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entidade = cx.entity();
        let caminho = self
            .plano
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string());
        let pode_gravar = self.estimativa.is_some();
        let deep = Button::new("profunda", "Profunda")
            .disabled("--deep ainda não está disponível no Moirai em Rust");
        let gravar = {
            let b = Button::new("gravar", "Gravar no plano").primary();
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
                        v.reestimar();
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
                        v.abrir_plano(p);
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
            .gap(px(14.0))
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

        let mut janelas = div().flex().gap(px(12.0));
        for j in JANELAS {
            let m = vm::medidor(j, est.limites.get(j));
            janelas = janelas.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
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
            .gap(px(14.0))
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

    fn tela_em_breve(tela: Tela) -> gpui::AnyElement {
        EmptyState::new(tela.rotulo())
            .hint("Esta tela chega na próxima entrega do app.")
            .into_any_element()
    }
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
        let rail = self.rail();
        let corpo = match self.tela {
            Tela::Estimar => self.tela_estimar(cx),
            Tela::Historico => self.tela_historico(cx),
            outra => Self::tela_em_breve(outra),
        };
        let mut raiz = div()
            .id("moirai")
            .track_focus(&self.foco)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if let Some(t) = vm::tela_da_tecla(&ev.keystroke.key) {
                    v.tela = t;
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
                    .min_h(px(0.0))
                    .child(rail)
                    .child(div().flex_1().p(px(16.0)).child(corpo)),
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
    let aberta = cx.open_window(gpui::WindowOptions::default(), |window, cx| {
        window.set_window_title(crate::NOME);
        let cwd = std::env::current_dir().unwrap_or_default();
        let view: Entity<MoiraiView> =
            cx.new(|cx| MoiraiView::new(Env::from_process(), cwd, cx.focus_handle()));
        view
    });
    if aberta.is_err() {
        cx.quit();
    }
}
