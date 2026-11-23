# Moirai

Moirai: estimativa e uso por janela dos agentes do Hades.

## Crates

- `crates/moirai-core`: regra de negócio, sem interface.
- `crates/moirai-cli`: binário `moirai` (clap).
- `crates/moirai-app`: view gpui exportada e binário `moirai-app`.

## Estado

CLI e app portados (estimar, histórico, teto, janelas, configurações); o app também abre embutido no painel do Hades (`FerramentaEmbutida` do Hades-UI v0.3.0).

## Relação com o Hades-Skill

Faz parte da reescrita em Rust das ferramentas do [Hades-Skill](https://github.com/The-Hades-Organization/Hades-Skill). O Hades-Skill continua sendo a fonte até o corte, quando passa a chamar o binário.

## Licença

MIT. Veja `LICENSE`.
