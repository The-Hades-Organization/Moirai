fn main() {
    let args: Vec<String> = std::env::args().collect();
    let escuro = args.iter().any(|a| a == "--dark");
    let tela = args
        .iter()
        .position(|a| a == "--tela")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| moirai_app::vm::Tela::de_nome(n));
    gpui_platform::application().run(move |cx| {
        let modo = if escuro {
            hades_ui::ThemeMode::Dark
        } else {
            hades_ui::ThemeMode::Light
        };
        hades_ui::init(cx, modo);
        moirai_app::abrir_janela_em(cx, tela);
    });
}
