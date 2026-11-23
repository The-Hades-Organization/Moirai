//! O Moirai dentro do painel do Hades: telas do trilho, comandos da paleta e a view sem trilho nem janela.

use std::path::PathBuf;

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div};
use hades_core::Env;
use hades_ui::{Comando, Embutida, FerramentaEmbutida, FerramentaInfo, IconName, TelaInfo};
use moirai_core::model::Mode;

use super::MoiraiView;
use crate::vm::Tela;

const fn tela_info(t: Tela) -> TelaInfo {
    TelaInfo {
        id: t.id(),
        rotulo: t.rotulo(),
        icone: t.icone(),
    }
}

/// As telas do trilho próprio, na mesma ordem.
const TELAS: &[TelaInfo] = &[
    tela_info(Tela::Estimar),
    tela_info(Tela::Historico),
    tela_info(Tela::Teto),
    tela_info(Tela::Janelas),
    tela_info(Tela::Config),
];

/// As telas quando as Configurações vêm do painel.
const TELAS_INTEGRADO: &[TelaInfo] = &[
    tela_info(Tela::Estimar),
    tela_info(Tela::Historico),
    tela_info(Tela::Teto),
    tela_info(Tela::Janelas),
];

const PROFUNDA: &str = "estimar.profunda";
const GRAVAR: &str = "estimar.gravar";
const MODO_PROJETO: &str = "estimar.modo-projeto";
const MODO_SKILL: &str = "estimar.modo-skill";

/// Os botões da tela Estimar plano.
const COMANDOS: &[Comando] = &[
    Comando {
        id: PROFUNDA,
        grupo: "Estimar plano",
        rotulo: "Estimativa profunda",
        descricao: Some("Estima cada unidade do plano aberto com o Claude"),
        icone: IconName::ScanSearch,
        atalho: None,
    },
    Comando {
        id: GRAVAR,
        grupo: "Estimar plano",
        rotulo: "Gravar no plano",
        descricao: Some("Troca a seção \"## Estimativa\" do plano aberto"),
        icone: IconName::Save,
        atalho: None,
    },
    Comando {
        id: MODO_PROJETO,
        grupo: "Estimar plano",
        rotulo: "Estimar como projeto",
        descricao: Some("Histórico de unidades de projeto"),
        icone: IconName::Package,
        atalho: None,
    },
    Comando {
        id: MODO_SKILL,
        grupo: "Estimar plano",
        rotulo: "Estimar como skill",
        descricao: Some("Histórico de unidades de skill"),
        icone: IconName::Puzzle,
        atalho: None,
    },
];

/// Telas que a view mostra com as opções `opts`.
pub fn telas(opts: Embutida) -> &'static [TelaInfo] {
    if opts.integrado {
        TELAS_INTEGRADO
    } else {
        TELAS
    }
}

impl MoiraiView {
    /// Tela de id `id` entre as visíveis; id desconhecido, ou Configurações com o painel integrado, dá a primeira.
    fn tela_visivel(&self, id: &str) -> Tela {
        telas(Embutida {
            integrado: self.integrado,
        })
        .iter()
        .find(|t| t.id == id)
        .and_then(|t| Tela::de_nome(t.id))
        .unwrap_or(Tela::Estimar)
    }
}

impl FerramentaEmbutida for MoiraiView {
    /// Ambiente do processo e pasta do projeto.
    type Ambiente = (Env, PathBuf);

    fn info() -> FerramentaInfo {
        FerramentaInfo {
            id: "moirai",
            titulo: crate::NOME,
            icone: IconName::Hourglass,
            telas: TELAS,
            comandos: COMANDOS,
        }
    }

    fn embutida(
        (env, cwd): Self::Ambiente,
        foco: Option<&str>,
        opts: Embutida,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::iniciar(cx);
        let mut v = Self::new(env, cwd, cx.focus_handle(), cx);
        v.integrado = opts.integrado;
        if let Some(id) = foco {
            let t = v.tela_visivel(id);
            v.mudar_tela(t, cx);
        }
        v
    }

    fn render_tela(
        &mut self,
        tela: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.tela_visivel(tela);
        if t != self.tela {
            self.mudar_tela(t, cx);
        }
        let corpo = self.corpo(t, window, cx);
        div()
            .size_full()
            .flex()
            .min_h_0()
            .child(corpo)
            .children(self.confirmacao(cx))
            .into_any_element()
    }

    fn executar(&mut self, comando: &str, _window: &mut Window, cx: &mut Context<Self>) {
        match comando {
            PROFUNDA => self.rodar_profunda(cx),
            GRAVAR if self.estimativa.is_some() => self.pedir_confirmacao(cx),
            MODO_PROJETO | MODO_SKILL => {
                self.modo = if comando == MODO_SKILL {
                    Mode::Skill
                } else {
                    Mode::Projeto
                };
                self.reestimar(cx);
                cx.notify();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_sem_ids_repetidos() {
        assert!(MoiraiView::info().ids_repetidos().is_empty());
    }

    #[test]
    fn toda_tela_do_trilho_tem_tela_info() {
        let info = MoiraiView::info();
        for t in Tela::TODAS {
            let tela = info
                .tela(t.id())
                .unwrap_or_else(|| panic!("sem TelaInfo: {}", t.rotulo()));
            assert_eq!((tela.rotulo, tela.icone), (t.rotulo(), t.icone()));
        }
        assert_eq!(info.telas.len(), Tela::TODAS.len());
    }

    #[test]
    fn ids_das_telas_valem_na_flag_tela() {
        for t in MoiraiView::info().telas {
            assert_eq!(Tela::de_nome(t.id).map(Tela::id), Some(t.id));
        }
    }

    #[test]
    fn integrado_tira_configuracoes() {
        let ids = |opts| telas(opts).iter().map(|t| t.id).collect::<Vec<_>>();
        assert!(ids(Embutida::default()).contains(&Tela::Config.id()));
        let integrado = ids(Embutida { integrado: true });
        assert!(!integrado.contains(&Tela::Config.id()));
        assert_eq!(integrado.len(), Tela::TODAS.len() - 1);
    }
}
