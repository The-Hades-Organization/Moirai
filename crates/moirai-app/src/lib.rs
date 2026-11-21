//! View gpui do Moirai, exportada para o app próprio e para o painel do Hades.

mod service;
mod view;
pub mod vm;

pub use view::{MoiraiView, abrir_janela};

pub const NOME: &str = "Moirai";
