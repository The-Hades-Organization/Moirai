fn main() {
    let escuro = std::env::args().any(|a| a == "--dark");
    gpui_platform::application().run(move |cx| {
        let modo = if escuro {
            hades_ui::ThemeMode::Dark
        } else {
            hades_ui::ThemeMode::Light
        };
        hades_ui::init(cx, modo);
        moirai_app::abrir_janela(cx);
    });
}
