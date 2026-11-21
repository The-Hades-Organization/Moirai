fn main() {
    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);
        moirai_app::abrir_janela(cx);
    });
}
