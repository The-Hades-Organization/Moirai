//! View gpui do Moirai, exportada para o app próprio e para o painel do Hades.

use gpui::{
    App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, div, rgb,
};

pub const NOME: &str = "Moirai";

pub struct MoiraiView;

impl Render for MoiraiView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .text_xl()
            .child(NOME)
    }
}

pub fn abrir_janela(cx: &mut App) {
    let aberta = cx.open_window(gpui::WindowOptions::default(), |window, cx| {
        window.set_window_title(NOME);
        cx.new(|_| MoiraiView)
    });
    if aberta.is_err() {
        cx.quit();
    }
}
