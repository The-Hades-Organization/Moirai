//! View gpui do Moirai, exportada para o app próprio e para o painel do Hades.

pub mod config;
mod service;
mod view;
pub mod vm;

pub use view::{MoiraiView, abrir_janela, abrir_janela_em};

pub const NOME: &str = "Moirai";
