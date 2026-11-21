//! Tipos da estimativa (`src/lib/moirai.ts:7-73`). Sem serde: o JSON entra com o `estimate` da CLI.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Skill,
    Projeto,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Skill => "skill",
            Mode::Projeto => "projeto",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Pequena,
    Media,
    Grande,
}

impl Band {
    pub fn as_str(self) -> &'static str {
        match self {
            Band::Pequena => "pequena",
            Band::Media => "media",
            Band::Grande => "grande",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Alta,
    Media,
    Baixa,
    Nenhuma,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Alta => "alta",
            Confidence::Media => "media",
            Confidence::Baixa => "baixa",
            Confidence::Nenhuma => "nenhuma",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Parecidas,
    Tamanho,
    Geral,
    SemHistorico,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::Parecidas => "parecidas",
            Basis::Tamanho => "tamanho",
            Basis::Geral => "geral",
            Basis::SemHistorico => "sem-historico",
        }
    }
}

/// `Params` (`src/lib/moirai.ts:19`). Inteiros do JS: `f64`, como no TS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub min_sample: f64,
    pub high_factor: f64,
    pub small_lines: f64,
    pub large_lines: f64,
}

/// `HistoryUnit` (`src/lib/moirai.ts:29`).
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryUnit {
    pub name: String,
    pub root: String,
    pub mode: Mode,
    pub role: String,
    pub state: String,
    pub created_at: String,
    pub done_at: String,
    pub cost_usd: Option<f64>,
    pub tokens: Option<f64>,
    pub wall_s: Option<f64>,
    pub gate_s: f64,
    pub gate_peak_mb: Option<f64>,
    pub clears: f64,
    pub resumes: Option<f64>,
    pub files: Option<f64>,
    pub lines: Option<f64>,
    pub band: Option<Band>,
}

/// `Spread` (`src/lib/moirai.ts:51`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub n: usize,
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
}

/// `UnitEstimate` (`src/lib/moirai.ts:58`).
#[derive(Clone, Debug, PartialEq)]
pub struct UnitEstimate {
    pub unit: String,
    pub role: String,
    pub mode: Mode,
    pub declared_lines: Option<f64>,
    pub band: Option<Band>,
    pub basis: Basis,
    pub sample: usize,
    pub confidence: Confidence,
    pub cost_usd: Option<Spread>,
    pub wall_min: Option<Spread>,
    pub gate_min: Option<Spread>,
    pub tokens: Option<Spread>,
    pub note: String,
}

/// `PlanUnit` (`src/lib/moirai.ts:71`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanUnit {
    pub name: String,
    pub role: String,
    pub scope: String,
}
